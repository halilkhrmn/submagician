//! Dropping folders and videos on the window under Wayland. winit 0.30 reports drops only on
//! X11, Windows and macOS, so here SubMagician listens on the same Wayland connection itself:
//! one `wl_data_device` per seat, accepting `text/uri-list` drags and reading the paths when
//! they are dropped.

use std::collections::HashMap;
use std::io::Read;
use std::os::fd::AsFd;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;

use wayland_client::backend::ObjectId;
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};

const URI_LIST: &str = "text/uri-list";

type OnDrop = Arc<dyn Fn(PathBuf) + Send + Sync>;

struct State {
    conn: Connection,
    manager: Option<WlDataDeviceManager>,
    seats: Vec<WlSeat>,
    devices: Vec<WlDataDevice>,
    /// Mime types announced for each offer.
    offers: HashMap<ObjectId, Vec<String>>,
    /// The drag over our window that carries file paths.
    current: Option<WlDataOffer>,
    on_drop: OnDrop,
}

/// Starts listening for drops on the Wayland display `display` (a `wl_display*` owned by the
/// windowing backend, alive for the whole program).
pub fn start(display: *mut std::ffi::c_void, on_drop: impl Fn(PathBuf) + Send + Sync + 'static) -> Result<(), String> {
    // SAFETY: the display belongs to winit, which keeps it open until the program exits.
    let backend = unsafe { wayland_client::backend::Backend::from_foreign_display(display.cast()) };
    let conn = Connection::from_backend(backend);
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut state = State {
        conn: conn.clone(),
        manager: None,
        seats: Vec::new(),
        devices: Vec::new(),
        offers: HashMap::new(),
        current: None,
        on_drop: Arc::new(on_drop),
    };
    queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
    let Some(manager) = state.manager.clone() else {
        return Err("the compositor has no wl_data_device_manager".into());
    };
    state.devices = state.seats.iter().map(|seat| manager.get_data_device(seat, &qh, ())).collect();
    queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name("wayland-drop".into())
        .spawn(move || while queue.blocking_dispatch(&mut state).is_ok() {})
        .map_err(|e| e.to_string())?;
    Ok(())
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_seat" => state.seats.push(registry.bind(name, version.min(5), qh, ())),
                "wl_data_device_manager" => state.manager = Some(registry.bind(name, version.min(3), qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(_: &mut Self, _: &WlSeat, _: <WlSeat as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlDataDeviceManager,
        _: <WlDataDeviceManager as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                state.offers.insert(id.id(), Vec::new());
            }
            wl_data_device::Event::Enter { serial, id, .. } => {
                state.current = None;
                let Some(offer) = id else { return };
                let has_paths = state.offers.get(&offer.id()).is_some_and(|m| m.iter().any(|m| m == URI_LIST));
                if has_paths {
                    offer.accept(serial, Some(URI_LIST.into()));
                    if offer.version() >= 3 {
                        offer.set_actions(DndAction::Copy, DndAction::Copy);
                    }
                    state.current = Some(offer);
                } else {
                    offer.accept(serial, None);
                }
            }
            wl_data_device::Event::Leave => {
                if let Some(offer) = state.current.take() {
                    state.offers.remove(&offer.id());
                    offer.destroy();
                }
            }
            wl_data_device::Event::Drop => {
                if let Some(offer) = state.current.take() {
                    state.offers.remove(&offer.id());
                    receive(&state.conn, offer, state.on_drop.clone());
                }
            }
            // Clipboard offers are not ours to read; forget them.
            wl_data_device::Event::Selection { id: Some(offer) } => {
                state.offers.remove(&offer.id());
            }
            _ => {}
        }
    }

    event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);
}

impl Dispatch<WlDataOffer, ()> for State {
    fn event(
        state: &mut Self,
        offer: &WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event
            && let Some(mimes) = state.offers.get_mut(&offer.id())
        {
            mimes.push(mime_type);
        }
    }
}

/// Asks the drag source for the paths and reads them on a separate thread, so a slow source
/// cannot block the Wayland queue.
fn receive(conn: &Connection, offer: WlDataOffer, on_drop: OnDrop) {
    let Ok((mut reader, writer)) = UnixStream::pair() else { return };
    offer.receive(URI_LIST.into(), writer.as_fd());
    let _ = conn.flush();
    drop(writer);
    let conn = conn.clone();
    std::thread::spawn(move || {
        let mut data = Vec::new();
        let _ = reader.read_to_end(&mut data);
        if offer.version() >= 3 {
            offer.finish();
        }
        offer.destroy();
        let _ = conn.flush();
        if let Some(path) = parse_uri_list(&data).into_iter().next() {
            on_drop(path);
        }
    });
}

/// `file://` URIs of a `text/uri-list` (RFC 2483) as paths; other schemes are skipped.
fn parse_uri_list(data: &[u8]) -> Vec<PathBuf> {
    String::from_utf8_lossy(data)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|uri| {
            let rest = uri.strip_prefix("file://")?;
            // "file:///path" or "file://localhost/path"; other hosts are not local files.
            let path = if rest.starts_with('/') { rest } else { rest.strip_prefix("localhost")? };
            Some(PathBuf::from(std::ffi::OsString::from_vec(percent_decode(path))))
        })
        .collect()
}

fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_uri_lists() {
        let list = b"# from Files\r\nfile:///home/me/Filmler/Sinek%20Adam%20%C4%B0zle.mkv\r\nfile://localhost/srv/a%2Fb\r\nhttps://example.com/x\r\nfile://other-host/x\r\n";
        assert_eq!(
            parse_uri_list(list),
            vec![PathBuf::from("/home/me/Filmler/Sinek Adam İzle.mkv"), PathBuf::from("/srv/a/b")]
        );
        assert!(parse_uri_list(b"").is_empty());
        assert_eq!(percent_decode("100%"), b"100%");
        assert_eq!(percent_decode("a%zzb"), b"a%zzb");
    }
}
