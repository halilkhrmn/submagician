-- SubMagician for VLC: finds a subtitle for the playing video in your languages, syncs it to the
-- audio and loads it. Installed and updated by SubMagician (Settings > Player plugins).
--
-- In VLC: View > SubMagician. It finds a subtitle for the video that is playing, and while it is
-- on, for every video that starts without a subtitle in your first language.

-- The SubMagician command-line tool (filled in by the installer).
local CLI = { @@CLI@@ }
local AUTO = @@AUTO@@

local dlg, label, last_uri

function descriptor()
    return {
        title = "SubMagician",
        version = "@@VERSION@@",
        author = "SubMagician",
        url = "https://github.com/halilkhrmn/submagician",
        shortdesc = "SubMagician: find a subtitle",
        description = "Finds a subtitle for the playing video in your languages, syncs it to the audio and loads it.",
        capabilities = { "input-listener", "menu" },
    }
end

local function is_windows()
    return package.config:sub(1, 1) == "\\"
end

local function quote(s)
    if is_windows() then return '"' .. s .. '"' end
    return "'" .. s:gsub("'", "'\\''") .. "'"
end

local function show(text)
    if not dlg then
        dlg = vlc.dialog("SubMagician")
        label = dlg:add_label("", 1, 1, 2, 1)
        dlg:add_button("Search again", function() find(true, false) end, 1, 2, 1, 1)
        dlg:add_button("Close", function() dlg:hide() end, 2, 2, 1, 1)
    end
    label:set_text(text)
    dlg:show()
    dlg:update()
end

function find(force, auto)
    local item = vlc.input.item()
    if not item then
        if not auto then show("Nothing is playing.") end
        return
    end
    local uri = item:uri()
    if not uri or not uri:find("^file://") then
        if not auto then show("SubMagician works with files on this computer.") end
        return
    end
    if not auto then show("Finding a subtitle…") end

    local parts = {}
    for _, a in ipairs(CLI) do parts[#parts + 1] = quote(a) end
    parts[#parts + 1] = "--player"
    if auto then parts[#parts + 1] = "--auto" end
    if force then parts[#parts + 1] = "--force" end
    parts[#parts + 1] = "--"
    parts[#parts + 1] = quote(uri)
    local command = table.concat(parts, " ")
    -- cmd.exe drops the outer quotes of "/c" commands that start with a quote.
    if is_windows() then command = '"' .. command .. '"' end

    local pipe = io.popen(command, "r")
    if not pipe then
        show("Could not run the SubMagician tool. Install the plugin again from SubMagician.")
        return
    end
    local subtitle, text
    for line in pipe:lines() do
        local kind, value = line:match("^(%a+)\t(.*)$")
        if kind == "subtitle" then subtitle = value end
        if kind == "message" then text = value end
    end
    pipe:close()

    if subtitle and vlc.input.item() and vlc.input.item():uri() == uri then
        vlc.input.add_subtitle(subtitle, true)
    end
    if text and (subtitle or not auto) then show(text) end
    if not text and not auto then show("The SubMagician tool did not answer. Install the plugin again from SubMagician.") end
    vlc.msg.info("[SubMagician] " .. (text or "no answer"))
end

function activate()
    local item = vlc.input.item()
    last_uri = item and item:uri()
    find(false, false)
end

function deactivate()
    if dlg then dlg:delete() end
    dlg = nil
end

function close()
    vlc.deactivate()
end

function menu()
    return { "Find a subtitle", "Search again" }
end

function trigger_menu(id)
    find(id == 2, false)
end

function input_changed()
    local item = vlc.input.item()
    local uri = item and item:uri()
    if AUTO and uri and uri ~= last_uri then
        last_uri = uri
        find(false, true)
    end
end
