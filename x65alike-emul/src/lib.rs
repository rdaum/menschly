use std::collections::VecDeque;

pub const SRAM_SIZE: usize = 2 * 1024 * 1024;
pub const CPU_SPACE_SIZE: usize = 16 * 1024 * 1024;

pub const RAMBLOCK_REG: u32 = 0x000000;
pub const ROMBLOCK_REG: u32 = 0x000001;

pub const IO_START: u32 = 0x009f00;
pub const IO_END: u32 = 0x009fff;

pub const RAMBLOCK_FRAME_START: u32 = 0x00a000;
pub const RAMBLOCK_FRAME_END: u32 = 0x00bfff;
pub const ROMBLOCK_FRAME_START: u32 = 0x00c000;
pub const ROMBLOCK_FRAME_END: u32 = 0x00ffff;

pub const SERIAL_DATA: u32 = 0x009f70;
pub const SERIAL_STATUS: u32 = 0x009f71;
pub const SERIAL_CONTROL: u32 = 0x009f72;

pub const SERIAL_STATUS_RX_READY: u8 = 0x01;
pub const SERIAL_STATUS_TX_READY: u8 = 0x02;

const RAMBLOCK_SIZE: usize = 0x2000;
const ROMBLOCK_SIZE: usize = 0x4000;
const ROMBLOCK_BASE: usize = 0x080000;
const PBL_SIZE: usize = 512;

#[derive(Debug, Clone)]
pub struct Machine {
    sram: Vec<u8>,
    pbl: [u8; PBL_SIZE],
    ramblock: u8,
    romblock: u8,
    serial_rx: VecDeque<u8>,
    serial_tx: Vec<u8>,
    serial_control: u8,
}

impl Machine {
    pub fn new() -> Self {
        Self {
            sram: vec![0; SRAM_SIZE],
            pbl: [0; PBL_SIZE],
            ramblock: 0,
            romblock: 0,
            serial_rx: VecDeque::new(),
            serial_tx: Vec::new(),
            serial_control: 0,
        }
    }

    pub fn load(&mut self, address: u32, bytes: &[u8]) {
        let start = self
            .physical_sram_address(address)
            .expect("load address should map to SRAM");
        let end = start + bytes.len();
        assert!(end <= self.sram.len(), "load should fit in SRAM");
        self.sram[start..end].copy_from_slice(bytes);
    }

    pub fn read(&mut self, address: u32) -> u8 {
        let address = address & 0x00ff_ffff;
        match self.decode(address) {
            Mapping::RamBlockReg => self.ramblock,
            Mapping::RomBlockReg => self.romblock,
            Mapping::SerialData => self.serial_rx.pop_front().unwrap_or(0),
            Mapping::SerialStatus => {
                SERIAL_STATUS_TX_READY
                    | if self.serial_rx.is_empty() {
                        0
                    } else {
                        SERIAL_STATUS_RX_READY
                    }
            }
            Mapping::SerialControl => self.serial_control,
            Mapping::Io => 0,
            Mapping::Sram(index) => self.sram[index],
            Mapping::Pbl(index) => self.pbl[index],
            Mapping::Unmapped => 0xff,
        }
    }

    pub fn write(&mut self, address: u32, value: u8) {
        let address = address & 0x00ff_ffff;
        match self.decode(address) {
            Mapping::RamBlockReg => self.ramblock = value,
            Mapping::RomBlockReg => self.romblock = value,
            Mapping::SerialData => self.serial_tx.push(value),
            Mapping::SerialControl => self.serial_control = value,
            Mapping::SerialStatus | Mapping::Io | Mapping::Pbl(_) | Mapping::Unmapped => {}
            Mapping::Sram(index) => self.sram[index] = value,
        }
    }

    pub fn push_serial_input(&mut self, bytes: &[u8]) {
        self.serial_rx.extend(bytes.iter().copied());
    }

    pub fn take_serial_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.serial_tx)
    }

    pub fn serial_output(&self) -> &[u8] {
        &self.serial_tx
    }

    pub fn byte(&self, address: u32) -> u8 {
        let Some(index) = self.physical_sram_address(address & 0x00ff_ffff) else {
            return 0xff;
        };
        self.sram[index]
    }

    pub fn ramblock(&self) -> u8 {
        self.ramblock
    }

    pub fn romblock(&self) -> u8 {
        self.romblock
    }

    fn decode(&self, address: u32) -> Mapping {
        match address {
            RAMBLOCK_REG => Mapping::RamBlockReg,
            ROMBLOCK_REG => Mapping::RomBlockReg,
            SERIAL_DATA => Mapping::SerialData,
            SERIAL_STATUS => Mapping::SerialStatus,
            SERIAL_CONTROL => Mapping::SerialControl,
            IO_START..=IO_END => Mapping::Io,
            RAMBLOCK_FRAME_START..=RAMBLOCK_FRAME_END => Mapping::Sram(
                (((self.ramblock ^ 0x80) as usize) * RAMBLOCK_SIZE)
                    + ((address - RAMBLOCK_FRAME_START) as usize),
            ),
            ROMBLOCK_FRAME_START..=ROMBLOCK_FRAME_END if self.romblock & 0x80 != 0 => {
                Mapping::Pbl(((address - ROMBLOCK_FRAME_START) as usize) % PBL_SIZE)
            }
            ROMBLOCK_FRAME_START..=ROMBLOCK_FRAME_END => Mapping::Sram(
                ROMBLOCK_BASE
                    + ((self.romblock as usize & 0x1f) * ROMBLOCK_SIZE)
                    + ((address - ROMBLOCK_FRAME_START) as usize),
            ),
            _ => self
                .physical_sram_address(address)
                .map_or(Mapping::Unmapped, Mapping::Sram),
        }
    }

    fn physical_sram_address(&self, address: u32) -> Option<usize> {
        if address < SRAM_SIZE as u32 {
            Some(address as usize)
        } else {
            None
        }
    }
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mapping {
    RamBlockReg,
    RomBlockReg,
    SerialData,
    SerialStatus,
    SerialControl,
    Io,
    Sram(usize),
    Pbl(usize),
    Unmapped,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_ramblock_frame_with_x65_inverted_high_bit() {
        let mut machine = Machine::new();

        machine.write(RAMBLOCK_REG, 0);
        machine.write(RAMBLOCK_FRAME_START, 0x42);
        assert_eq!(machine.byte(0x100000), 0x42);

        machine.write(RAMBLOCK_REG, 0x80);
        machine.write(RAMBLOCK_FRAME_START, 0x55);
        assert_eq!(machine.byte(0x000000), 0x55);
    }

    #[test]
    fn maps_romblock_frame_into_middle_sram_region() {
        let mut machine = Machine::new();

        machine.write(ROMBLOCK_REG, 1);
        machine.write(ROMBLOCK_FRAME_START, 0x66);

        assert_eq!(machine.byte(0x084000), 0x66);
    }

    #[test]
    fn serial_status_tracks_scripted_input_and_output_is_buffered() {
        let mut machine = Machine::new();

        assert_eq!(machine.read(SERIAL_STATUS), SERIAL_STATUS_TX_READY);
        machine.push_serial_input(b"A");
        assert_eq!(
            machine.read(SERIAL_STATUS),
            SERIAL_STATUS_TX_READY | SERIAL_STATUS_RX_READY
        );
        assert_eq!(machine.read(SERIAL_DATA), b'A');
        assert_eq!(machine.read(SERIAL_STATUS), SERIAL_STATUS_TX_READY);

        machine.write(SERIAL_DATA, b'!');
        assert_eq!(machine.serial_output(), b"!");
    }
}
