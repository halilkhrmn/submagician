//! HTTP clients. Certificates are checked against the system's roots; a system without any (a
//! minimal Linux install or container without ca-certificates) cannot build such a client, and
//! then the Mozilla roots built into SubMagician are used instead of failing.

use std::time::Duration;

use reqwest::{Certificate, Client, ClientBuilder};

use crate::Result;

/// A client with SubMagician's user agent; `timeout` limits whole requests (`None` for
/// downloads of any size).
pub(crate) fn client(timeout: Option<Duration>) -> Result<Client> {
    match builder(timeout).build() {
        Ok(client) => Ok(client),
        Err(e) => {
            log::warn!("system certificates not usable ({e}); using the built-in ones");
            Ok(builder(timeout).tls_certs_only(builtin_roots()).build()?)
        }
    }
}

fn builder(timeout: Option<Duration>) -> ClientBuilder {
    let b = Client::builder().user_agent(crate::provider::user_agent()).connect_timeout(Duration::from_secs(30));
    match timeout {
        Some(t) => b.timeout(t),
        None => b,
    }
}

fn builtin_roots() -> Vec<Certificate> {
    webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().filter_map(|c| Certificate::from_der(c.as_ref()).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_roots_make_a_client() {
        let roots = builtin_roots();
        assert!(roots.len() > 100, "{} roots", roots.len());
        builder(Some(Duration::from_secs(5))).tls_certs_only(roots).build().expect("client with built-in roots");
        client(None).expect("client");
    }
}
