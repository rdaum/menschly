-- Captures one sound command's chip writes, from a fresh boot.
--
--   CMD=0F AT=150 FRAMES=2500 OUT=0f.log ./sound.sh 60
--
-- (capture_sound.py runs it for every command.) It stops QUIET frames after
-- the last write to either chip other than the timer flags (YM2203 $27),
-- or at FRAMES.
--
-- At frame AT the main CPU is parked (PARK=0 leaves it running) and the
-- command is queued as the main CPU's NMI handler does (see cmd.lua). Every write to the YM3526 ($A000/$A001, "opl") and the YM2203
-- ($9000/$9001, "opn") is logged as "<frame> <chip> <reg> <value>" (hex),
-- the register being the last one selected; every write to the command
-- queue's head ($8F80) as "<frame> queue <value>", so commands the main CPU
-- sends of its own show up. The taps only format numbers (reading the frame
-- counter inside one wedges MAME).
-- MAME may run an autoboot script twice; only the first run sets up.
if _G.reglog_started then return end
_G.reglog_started = true
local CMD = tonumber(os.getenv("CMD") or "0F", 16)
local AT = tonumber(os.getenv("AT") or "150")
local FRAMES = tonumber(os.getenv("FRAMES") or "2500")
local QUIET = tonumber(os.getenv("QUIET") or "180")
local last_write = 0
local out = assert(io.open(assert(os.getenv("OUT"), "OUT"), "w"))
local programs = manager.machine.devices[":audiocpu"].spaces["program"]
local frame = 0
local selected = { opl = 0, opn = 0 }

local function chip_tap(chip)
  return function(offset, data, mask)
    if offset & 1 == 0 then
      selected[chip] = data
    else
      if chip == "opl" or selected[chip] ~= 0x27 then last_write = frame end
      out:write(frame .. " " .. chip .. " " .. string.format("%02x %02x", selected[chip], data) .. "\n")
    end
  end
end
local opl = programs:install_write_tap(0xa000, 0xa001, "opl", chip_tap("opl"))
local opn = programs:install_write_tap(0x9000, 0x9001, "opn", chip_tap("opn"))
local queue = programs:install_write_tap(0x8f80, 0x8f80, "queue", function(offset, data, mask)
  out:write(frame .. " queue " .. string.format("%02x", data) .. "\n")
end)
-- A tap lasts only while its handle does: keep them from the collector.
_G.reglog_taps = { opl, opn, queue }

local function send(cmd)
  local head = programs:read_u8(0x8f80)
  local tail = programs:read_u8(0x8f81)
  local slot = (head + 1) & 0x0f
  if slot == (tail & 0x0f) then tail = (tail + 1) & 0x0f end
  programs:write_u8(0x8f82 + slot, cmd)
  programs:write_u8(0x8f80, slot)
  programs:write_u8(0x8f81, tail)
end

-- Parks the main CPU in spare RAM with interrupts off, in a loop that only
-- feeds the watchdog ("ld ($fa80),a / jr" back), so it sends no commands of
-- its own and nothing resets; the sound CPU plays on by itself.
local function park_main()
  local main = manager.machine.devices[":maincpu"]
  local space = main.spaces["program"]
  for i, byte in ipairs({ 0x32, 0x80, 0xfa, 0x18, 0xfb }) do
    space:write_u8(0xf7ef + i, byte)
  end
  main.state["IFF1"].value = 0
  main.state["IFF2"].value = 0
  main.state["PC"].value = 0xf7f0
end

emu.register_frame_done(function()
  frame = frame + 1
  if frame == AT then
    out:write(frame .. " send " .. string.format("%02x", CMD) .. "\n")
    if os.getenv("PARK") ~= "0" then park_main() end
    send(CMD)
  end
  if frame >= FRAMES or (frame > AT + QUIET and frame - last_write > QUIET) then
    out:close()
    manager.machine:exit()
  end
end)
