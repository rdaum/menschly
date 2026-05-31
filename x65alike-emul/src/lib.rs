use std::collections::VecDeque;

pub const SRAM_SIZE: usize = 2 * 1024 * 1024;
pub const CPU_SPACE_SIZE: usize = 16 * 1024 * 1024;

pub const RAMBLOCK_ZP_MIRROR: u32 = 0x000000;
pub const ROMBLOCK_ZP_MIRROR: u32 = 0x000001;

pub const IO_START: u32 = 0x009f00;
pub const IO_END: u32 = 0x009fff;

pub const RAMBLOCK_FRAME_START: u32 = 0x00a000;
pub const RAMBLOCK_FRAME_END: u32 = 0x00bfff;
pub const ROMBLOCK_FRAME_START: u32 = 0x00c000;
pub const ROMBLOCK_FRAME_END: u32 = 0x00ffff;

pub const RAMBLOCK_REG: u32 = 0x009f50;
pub const ROMBLOCK_REG: u32 = 0x009f51;
pub const RAMBMASK_REG: u32 = 0x009f52;
pub const RMBCTRL_REG: u32 = 0x009f53;

pub const RMBCTRL_ENABLE_RAM_CD: u8 = 0x08;
pub const RMBCTRL_ENABLE_ROM_CDEF: u8 = 0x10;
pub const RMBCTRL_MIRROR_ZP: u8 = 0x20;
pub const RMBCTRL_AUTO_UNMAP_BOOTROM: u8 = 0x40;
pub const RMBCTRL_MAP_BOOTROM: u8 = 0x80;

pub const USB_UART_CTRL: u32 = 0x009f55;
pub const USB_UART_STAT: u32 = 0x009f56;
pub const USB_UART_DATA: u32 = 0x009f57;

pub const USB_UART_CTRL_IRQ_RX_NOT_EMPTY: u8 = 0x40;
pub const USB_UART_CTRL_IRQ_TX_NOT_FULL: u8 = 0x80;

pub const USB_UART_STAT_RX_FULL: u8 = 0x02;
pub const USB_UART_STAT_TX_EMPTY_SENT: u8 = 0x04;
pub const USB_UART_STAT_TX_FULL: u8 = 0x08;
pub const USB_UART_STAT_FRAME_ERROR: u8 = 0x10;
pub const USB_UART_STAT_PARITY_ERROR: u8 = 0x20;
pub const USB_UART_STAT_CTS_INACTIVE: u8 = 0x40;
pub const USB_UART_STAT_RX_EMPTY: u8 = 0x80;

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
    rambmask: u8,
    rmbctrl: u8,
    serial_rx: VecDeque<u8>,
    serial_tx: Vec<u8>,
    usb_uart_ctrl: u8,
    usb_uart_errors: u8,
}

impl Machine {
    pub fn new() -> Self {
        Self {
            sram: vec![0; SRAM_SIZE],
            pbl: [0; PBL_SIZE],
            ramblock: 0,
            romblock: 0,
            rambmask: 0xff,
            rmbctrl: RMBCTRL_ENABLE_ROM_CDEF,
            serial_rx: VecDeque::new(),
            serial_tx: Vec::new(),
            usb_uart_ctrl: 0x04,
            usb_uart_errors: 0,
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
            Mapping::RamBlockMaskReg => self.rambmask,
            Mapping::RamBlockControlReg => self.rmbctrl,
            Mapping::UsbUartData => self.serial_rx.pop_front().unwrap_or(0),
            Mapping::UsbUartStatus => self.usb_uart_status(),
            Mapping::UsbUartControl => self.usb_uart_ctrl,
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
            Mapping::RamBlockMaskReg => self.rambmask = value,
            Mapping::RamBlockControlReg => self.rmbctrl = value & !0x01,
            Mapping::UsbUartData => self.serial_tx.push(value),
            Mapping::UsbUartControl => self.usb_uart_ctrl = value,
            Mapping::UsbUartStatus => {
                self.usb_uart_errors &=
                    !(value & (USB_UART_STAT_FRAME_ERROR | USB_UART_STAT_PARITY_ERROR));
            }
            Mapping::Io | Mapping::Pbl(_) | Mapping::Unmapped => {}
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

    pub fn irq_pending(&self) -> bool {
        self.usb_uart_irq_pending()
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

    pub fn rambmask(&self) -> u8 {
        self.rambmask
    }

    pub fn rmbctrl(&self) -> u8 {
        self.rmbctrl
    }

    fn usb_uart_status(&self) -> u8 {
        USB_UART_STAT_TX_EMPTY_SENT
            | self.usb_uart_errors
            | if self.serial_rx.is_empty() {
                USB_UART_STAT_RX_EMPTY
            } else {
                0
            }
    }

    fn usb_uart_irq_pending(&self) -> bool {
        let rx_not_empty =
            self.usb_uart_ctrl & USB_UART_CTRL_IRQ_RX_NOT_EMPTY != 0 && !self.serial_rx.is_empty();
        let tx_not_full =
            self.usb_uart_ctrl & USB_UART_CTRL_IRQ_TX_NOT_FULL != 0 && !self.usb_uart_tx_full();
        rx_not_empty || tx_not_full
    }

    fn usb_uart_tx_full(&self) -> bool {
        false
    }

    fn decode(&self, address: u32) -> Mapping {
        match address {
            RAMBLOCK_ZP_MIRROR if self.rmbctrl & RMBCTRL_MIRROR_ZP != 0 => {
                Mapping::RamBlockReg
            }
            ROMBLOCK_ZP_MIRROR if self.rmbctrl & RMBCTRL_MIRROR_ZP != 0 => {
                Mapping::RomBlockReg
            }
            RAMBLOCK_REG => Mapping::RamBlockReg,
            ROMBLOCK_REG => Mapping::RomBlockReg,
            RAMBMASK_REG => Mapping::RamBlockMaskReg,
            RMBCTRL_REG => Mapping::RamBlockControlReg,
            USB_UART_DATA => Mapping::UsbUartData,
            USB_UART_STAT => Mapping::UsbUartStatus,
            USB_UART_CTRL => Mapping::UsbUartControl,
            IO_START..=IO_END => Mapping::Io,
            RAMBLOCK_FRAME_START..=RAMBLOCK_FRAME_END => Mapping::Sram(
                ((((self.ramblock & self.rambmask) ^ 0x80) as usize) * RAMBLOCK_SIZE)
                    + ((address - RAMBLOCK_FRAME_START) as usize),
            ),
            ROMBLOCK_FRAME_START..=ROMBLOCK_FRAME_END if self.bootrom_mapped() => {
                Mapping::Pbl(((address - ROMBLOCK_FRAME_START) as usize) % PBL_SIZE)
            }
            ROMBLOCK_FRAME_START..=ROMBLOCK_FRAME_END if self.romblock_frame_enabled() => {
                Mapping::Sram(self.romblock_physical_address(address))
            }
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

    fn bootrom_mapped(&self) -> bool {
        self.rmbctrl & RMBCTRL_MAP_BOOTROM != 0
    }

    fn romblock_frame_enabled(&self) -> bool {
        self.rmbctrl & RMBCTRL_ENABLE_ROM_CDEF != 0
    }

    fn romblock_physical_address(&self, address: u32) -> usize {
        ROMBLOCK_BASE
            + ((self.romblock as usize & 0x1f) * ROMBLOCK_SIZE)
            + ((address - ROMBLOCK_FRAME_START) as usize)
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
    RamBlockMaskReg,
    RamBlockControlReg,
    UsbUartData,
    UsbUartStatus,
    UsbUartControl,
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
    fn ramblock_mask_limits_ramblock_frame_selection() {
        let mut machine = Machine::new();

        machine.write(RAMBMASK_REG, 0x7f);
        machine.write(RAMBLOCK_REG, 0xff);
        machine.write(RAMBLOCK_FRAME_START, 0x33);

        assert_eq!(machine.rambmask(), 0x7f);
        assert_eq!(machine.byte(0x1fe000), 0x33);
    }

    #[test]
    fn zero_page_block_register_mirrors_require_rmbctrl_bit() {
        let mut machine = Machine::new();

        machine.write(RAMBLOCK_ZP_MIRROR, 0x12);
        machine.write(ROMBLOCK_ZP_MIRROR, 0x34);
        assert_eq!(machine.ramblock(), 0);
        assert_eq!(machine.romblock(), 0);
        assert_eq!(machine.byte(RAMBLOCK_ZP_MIRROR), 0x12);
        assert_eq!(machine.byte(ROMBLOCK_ZP_MIRROR), 0x34);

        machine.write(RMBCTRL_REG, machine.rmbctrl() | RMBCTRL_MIRROR_ZP);
        machine.write(RAMBLOCK_ZP_MIRROR, 0x56);
        machine.write(ROMBLOCK_ZP_MIRROR, 0x78);

        assert_eq!(machine.ramblock(), 0x56);
        assert_eq!(machine.romblock(), 0x78);
        assert_eq!(machine.read(RAMBLOCK_ZP_MIRROR), 0x56);
        assert_eq!(machine.read(ROMBLOCK_ZP_MIRROR), 0x78);
    }

    #[test]
    fn maps_romblock_frame_into_middle_sram_region() {
        let mut machine = Machine::new();

        machine.write(ROMBLOCK_REG, 1);
        machine.write(ROMBLOCK_FRAME_START, 0x66);

        assert_eq!(machine.byte(0x084000), 0x66);
    }

    #[test]
    fn usb_uart_status_tracks_scripted_input_and_output_is_buffered() {
        let mut machine = Machine::new();

        assert_eq!(
            machine.read(USB_UART_STAT),
            USB_UART_STAT_TX_EMPTY_SENT | USB_UART_STAT_RX_EMPTY
        );
        machine.push_serial_input(b"A");
        assert_eq!(
            machine.read(USB_UART_STAT),
            USB_UART_STAT_TX_EMPTY_SENT
        );
        assert_eq!(machine.read(USB_UART_DATA), b'A');
        assert_eq!(
            machine.read(USB_UART_STAT),
            USB_UART_STAT_TX_EMPTY_SENT | USB_UART_STAT_RX_EMPTY
        );

        machine.write(USB_UART_DATA, b'!');
        assert_eq!(machine.serial_output(), b"!");
    }

    #[test]
    fn usb_uart_control_asserts_irq_for_enabled_ready_conditions() {
        let mut machine = Machine::new();

        assert!(!machine.irq_pending());
        machine.write(USB_UART_CTRL, USB_UART_CTRL_IRQ_RX_NOT_EMPTY);
        assert!(!machine.irq_pending());

        machine.push_serial_input(b"A");
        assert!(machine.irq_pending());
        assert_eq!(machine.read(USB_UART_DATA), b'A');
        assert!(!machine.irq_pending());

        machine.write(USB_UART_CTRL, USB_UART_CTRL_IRQ_TX_NOT_FULL);
        assert!(machine.irq_pending());
    }
}
