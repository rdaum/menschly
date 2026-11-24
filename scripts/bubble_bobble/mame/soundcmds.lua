-- probe.lua, also logging (as "S <frame> <command>") every sound command
-- the main CPU queues for the sound CPU.
--   INPUTS: "frame:field:value,..." on top of the coin/start/button sequence
--           that starts round 1 (fields of :IN1, e.g. "P1 Right").
--   RANGES: "start-end,..." hex ranges of main-CPU memory to log.
--   OUT: TSV path; FIRST/LAST: frames to log; SNAPS: "frame,frame,...".
local OUT = assert(os.getenv("OUT"), "OUT")
local FIRST = tonumber(os.getenv("FIRST") or "2370")
local LAST = tonumber(os.getenv("LAST") or "3000")
local script = {
  [1200] = { { ":IN0", "Coin 1", 1 } }, [1215] = { { ":IN0", "Coin 1", 0 } },
  [1500] = { { ":IN1", "1 Player Start", 1 } }, [1515] = { { ":IN1", "1 Player Start", 0 } },
  [1800] = { { ":IN1", "P1 Button 1", 1 } }, [1815] = { { ":IN1", "P1 Button 1", 0 } },
}
for fr, field, v in string.gmatch(os.getenv("INPUTS") or "", "(%d+):([^:,]+):(%d)") do
  fr = tonumber(fr)
  script[fr] = script[fr] or {}
  table.insert(script[fr], { ":IN1", field, tonumber(v) })
end
local ranges = {}
for a, b in string.gmatch(os.getenv("RANGES") or "F440-F47F", "(%x+)-(%x+)") do
  ranges[#ranges + 1] = { tonumber(a, 16), tonumber(b, 16) }
end
local snaps = {}
for fr in string.gmatch(os.getenv("SNAPS") or "", "(%d+)") do snaps[tonumber(fr)] = true end
local f = io.open(OUT, "w")
-- Commands the main CPU sends the sound CPU, as its NMI handler queues them.
local sound_space = manager.machine.devices[":audiocpu"].spaces["program"]
local frame_now = 0
_G.sound_tap = sound_space:install_write_tap(0x8f82, 0x8f91, "cmds", function(offset, data, mask)
  f:write("S " .. frame_now .. " " .. string.format("%02x", data) .. "\n")
end)
local frame = 0
emu.register_frame_done(function()
  frame = frame + 1
  frame_now = frame
  for _, s in ipairs(script[frame] or {}) do
    manager.machine.ioport.ports[s[1]].fields[s[2]]:set_value(s[3])
  end
  if snaps[frame] then manager.machine.video:snapshot() end
  if frame < FIRST or frame > LAST then return end
  local m = manager.machine.devices[":maincpu"].spaces["program"]
  local t = { frame }
  for _, r in ipairs(ranges) do
    local h = {}
    for a = r[1], r[2] do h[#h + 1] = string.format("%02X", m:read_u8(a)) end
    t[#t + 1] = table.concat(h)
  end
  f:write(table.concat(t, "\t"), "\n"); f:flush()
end)
