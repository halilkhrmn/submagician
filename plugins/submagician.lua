-- SubMagician for mpv: finds a subtitle for the playing video in your languages, syncs it to the
-- audio and loads it. Installed and updated by SubMagician (Settings > Player plugins).
--
--   Alt+s        find a subtitle (uses the one next to the video when there is one)
--   Alt+Shift+s  search again, even when there is one
--
-- With auto=yes a video that starts without a subtitle in your first language gets one
-- automatically. Options go in script-opts/submagician.conf (auto=no to switch that off).
-- Keys can be changed in input.conf: "Alt+s script-binding submagician/find".

local mp = require "mp"
local msg = require "mp.msg"
local utils = require "mp.utils"
local options = require "mp.options"

-- The SubMagician command-line tool (filled in by the installer).
local CLI = { @@CLI@@ }

local opts = { auto = @@AUTO@@ }
options.read_options(opts, "submagician")

local busy = false

local function local_path()
    local path = mp.get_property("path")
    if not path then return nil end
    if path:find("^file://") then return path end
    if path:find("^%a[%w+.-]*://") then return nil end -- a stream
    return utils.join_path(mp.get_property("working-directory", ""), path)
end

local function load(subtitle)
    for _, track in ipairs(mp.get_property_native("track-list", {})) do
        if track.type == "sub" and track["external-filename"] == subtitle then
            mp.set_property_number("sid", track.id)
            return
        end
    end
    mp.commandv("sub-add", subtitle, "select")
end

local function run(force, auto)
    if busy then
        if not auto then mp.osd_message("SubMagician: still working…", 3) end
        return
    end
    local path = local_path()
    if not path then
        if not auto then mp.osd_message("SubMagician: only files on this computer", 3) end
        return
    end
    local args = {}
    for _, a in ipairs(CLI) do args[#args + 1] = a end
    args[#args + 1] = "--player"
    if auto then args[#args + 1] = "--auto" end
    if force then args[#args + 1] = "--force" end
    args[#args + 1] = "--"
    args[#args + 1] = path

    busy = true
    if not auto then mp.osd_message("SubMagician: finding a subtitle…", 30) end
    local playing = mp.get_property("path")
    mp.command_native_async({ name = "subprocess", args = args, capture_stdout = true, playback_only = false },
        function(success, result, err)
            busy = false
            if not success or not result then
                msg.error("could not run SubMagician: " .. tostring(err))
                mp.osd_message("SubMagician: could not run the SubMagician tool (reinstall the plugin)", 6)
                return
            end
            local subtitle, text
            for line in (result.stdout or ""):gmatch("[^\r\n]+") do
                local kind, value = line:match("^(%a+)\t(.*)$")
                if kind == "subtitle" then subtitle = value end
                if kind == "message" then text = value end
            end
            if result.status ~= 0 and not text then
                text = "the SubMagician tool failed (exit " .. tostring(result.status) .. ")"
            end
            -- The user may have moved on to another video meanwhile.
            if subtitle and mp.get_property("path") == playing then load(subtitle) end
            if text and (subtitle or not auto) then mp.osd_message("SubMagician: " .. text, 5) end
            if text then msg.info(text) end
        end)
end

mp.add_key_binding("Alt+s", "find", function() run(false, false) end)
mp.add_key_binding("Alt+S", "search-again", function() run(true, false) end)

mp.register_event("file-loaded", function()
    if opts.auto then run(false, true) end
end)
