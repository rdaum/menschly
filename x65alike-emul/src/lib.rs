use std::collections::VecDeque;

pub mod image;

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

pub const BLOCK_LBA0: u32 = 0x009f60;
pub const BLOCK_LBA1: u32 = 0x009f61;
pub const BLOCK_LBA2: u32 = 0x009f62;
pub const BLOCK_LBA3: u32 = 0x009f63;
pub const BLOCK_INDEX_LO: u32 = 0x009f64;
pub const BLOCK_INDEX_HI: u32 = 0x009f65;
pub const BLOCK_DATA: u32 = 0x009f66;
pub const BLOCK_COMMAND: u32 = 0x009f67;
pub const BLOCK_STATUS: u32 = 0x009f68;
pub const BLOCK_ERROR: u32 = 0x009f69;
pub const BLOCK_SECTOR_COUNT0: u32 = 0x009f6a;
pub const BLOCK_SECTOR_COUNT1: u32 = 0x009f6b;
pub const BLOCK_SECTOR_COUNT2: u32 = 0x009f6c;
pub const BLOCK_SECTOR_COUNT3: u32 = 0x009f6d;

pub const BLOCK_COMMAND_READ: u8 = 0x01;
pub const BLOCK_COMMAND_WRITE: u8 = 0x02;
pub const BLOCK_COMMAND_CLEAR_ERROR: u8 = 0x7f;

pub const BLOCK_STATUS_PRESENT: u8 = 0x01;
pub const BLOCK_STATUS_READY: u8 = 0x02;
pub const BLOCK_STATUS_ERROR: u8 = 0x04;
pub const BLOCK_STATUS_WRITE_PROTECT: u8 = 0x08;

pub const BLOCK_ERROR_NONE: u8 = 0x00;
pub const BLOCK_ERROR_NO_MEDIA: u8 = 0x01;
pub const BLOCK_ERROR_OUT_OF_RANGE: u8 = 0x02;
pub const BLOCK_ERROR_WRITE_PROTECT: u8 = 0x03;
pub const BLOCK_ERROR_BAD_COMMAND: u8 = 0xff;

const RAMBLOCK_SIZE: usize = 0x2000;
const ROMBLOCK_SIZE: usize = 0x4000;
const ROMBLOCK_BASE: usize = 0x080000;
const PBL_SIZE: usize = 512;
pub const BLOCK_SECTOR_SIZE: usize = 512;

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
    block_device: BlockDevice,
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
            block_device: BlockDevice::new(),
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

    pub fn load_cpu_visible(&mut self, address: u32, bytes: &[u8]) {
        for (offset, byte) in bytes.iter().copied().enumerate() {
            self.write(address.wrapping_add(offset as u32), byte);
        }
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
            Mapping::BlockRegister(register) => self.block_device.read_register(register),
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
            Mapping::BlockRegister(register) => self.block_device.write_register(register, value),
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

    pub fn attach_block_device(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        self.block_device.attach(bytes)
    }

    pub fn attach_write_protected_block_device(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        self.block_device.attach_write_protected(bytes)
    }

    pub fn detach_block_device(&mut self) {
        self.block_device.detach();
    }

    pub fn block_device_image(&self) -> Option<&[u8]> {
        self.block_device.image()
    }

    pub fn take_block_device_image(&mut self) -> Option<Vec<u8>> {
        self.block_device.take_image()
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
            BLOCK_LBA0 => Mapping::BlockRegister(BlockRegister::Lba0),
            BLOCK_LBA1 => Mapping::BlockRegister(BlockRegister::Lba1),
            BLOCK_LBA2 => Mapping::BlockRegister(BlockRegister::Lba2),
            BLOCK_LBA3 => Mapping::BlockRegister(BlockRegister::Lba3),
            BLOCK_INDEX_LO => Mapping::BlockRegister(BlockRegister::IndexLo),
            BLOCK_INDEX_HI => Mapping::BlockRegister(BlockRegister::IndexHi),
            BLOCK_DATA => Mapping::BlockRegister(BlockRegister::Data),
            BLOCK_COMMAND => Mapping::BlockRegister(BlockRegister::Command),
            BLOCK_STATUS => Mapping::BlockRegister(BlockRegister::Status),
            BLOCK_ERROR => Mapping::BlockRegister(BlockRegister::Error),
            BLOCK_SECTOR_COUNT0 => Mapping::BlockRegister(BlockRegister::SectorCount0),
            BLOCK_SECTOR_COUNT1 => Mapping::BlockRegister(BlockRegister::SectorCount1),
            BLOCK_SECTOR_COUNT2 => Mapping::BlockRegister(BlockRegister::SectorCount2),
            BLOCK_SECTOR_COUNT3 => Mapping::BlockRegister(BlockRegister::SectorCount3),
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

#[derive(Debug, Clone)]
struct BlockDevice {
    image: Option<Vec<u8>>,
    buffer: [u8; BLOCK_SECTOR_SIZE],
    lba: u32,
    index: u16,
    error: u8,
    write_protected: bool,
}

impl BlockDevice {
    fn new() -> Self {
        Self {
            image: None,
            buffer: [0; BLOCK_SECTOR_SIZE],
            lba: 0,
            index: 0,
            error: BLOCK_ERROR_NONE,
            write_protected: false,
        }
    }

    fn attach(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        self.attach_with_write_protect(bytes, false)
    }

    fn attach_write_protected(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        self.attach_with_write_protect(bytes, true)
    }

    fn attach_with_write_protect(&mut self, bytes: Vec<u8>, write_protected: bool) -> Result<(), String> {
        if bytes.len() % BLOCK_SECTOR_SIZE != 0 {
            return Err(format!(
                "block device image size {} is not a multiple of {} bytes",
                bytes.len(),
                BLOCK_SECTOR_SIZE
            ));
        }
        self.image = Some(bytes);
        self.buffer = [0; BLOCK_SECTOR_SIZE];
        self.lba = 0;
        self.index = 0;
        self.error = BLOCK_ERROR_NONE;
        self.write_protected = write_protected;
        Ok(())
    }

    fn detach(&mut self) {
        self.image = None;
        self.buffer = [0; BLOCK_SECTOR_SIZE];
        self.lba = 0;
        self.index = 0;
        self.error = BLOCK_ERROR_NO_MEDIA;
        self.write_protected = false;
    }

    fn image(&self) -> Option<&[u8]> {
        self.image.as_deref()
    }

    fn take_image(&mut self) -> Option<Vec<u8>> {
        self.image.take()
    }

    fn read_register(&mut self, register: BlockRegister) -> u8 {
        match register {
            BlockRegister::Lba0 => self.lba.to_le_bytes()[0],
            BlockRegister::Lba1 => self.lba.to_le_bytes()[1],
            BlockRegister::Lba2 => self.lba.to_le_bytes()[2],
            BlockRegister::Lba3 => self.lba.to_le_bytes()[3],
            BlockRegister::IndexLo => self.index.to_le_bytes()[0],
            BlockRegister::IndexHi => self.index.to_le_bytes()[1] & 0x01,
            BlockRegister::Data => {
                let value = self.buffer[usize::from(self.index & 0x01ff)];
                self.index = (self.index + 1) & 0x01ff;
                value
            }
            BlockRegister::Command => 0,
            BlockRegister::Status => self.status(),
            BlockRegister::Error => self.error,
            BlockRegister::SectorCount0
            | BlockRegister::SectorCount1
            | BlockRegister::SectorCount2
            | BlockRegister::SectorCount3 => {
                let bytes = self.sector_count().to_le_bytes();
                bytes[register.sector_count_byte_index().unwrap()]
            }
        }
    }

    fn write_register(&mut self, register: BlockRegister, value: u8) {
        match register {
            BlockRegister::Lba0 => self.set_lba_byte(0, value),
            BlockRegister::Lba1 => self.set_lba_byte(1, value),
            BlockRegister::Lba2 => self.set_lba_byte(2, value),
            BlockRegister::Lba3 => self.set_lba_byte(3, value),
            BlockRegister::IndexLo => {
                self.index = (self.index & 0x0100) | u16::from(value);
            }
            BlockRegister::IndexHi => {
                self.index = (self.index & 0x00ff) | (u16::from(value & 0x01) << 8);
            }
            BlockRegister::Data => {
                self.buffer[usize::from(self.index & 0x01ff)] = value;
                self.index = (self.index + 1) & 0x01ff;
            }
            BlockRegister::Command => self.run_command(value),
            BlockRegister::Error => {
                if value == 0 {
                    self.error = BLOCK_ERROR_NONE;
                }
            }
            BlockRegister::Status
            | BlockRegister::SectorCount0
            | BlockRegister::SectorCount1
            | BlockRegister::SectorCount2
            | BlockRegister::SectorCount3 => {}
        }
    }

    fn status(&self) -> u8 {
        let mut status = BLOCK_STATUS_READY;
        if self.image.is_some() {
            status |= BLOCK_STATUS_PRESENT;
        }
        if self.error != BLOCK_ERROR_NONE {
            status |= BLOCK_STATUS_ERROR;
        }
        if self.write_protected {
            status |= BLOCK_STATUS_WRITE_PROTECT;
        }
        status
    }

    fn sector_count(&self) -> u32 {
        let Some(image) = &self.image else {
            return 0;
        };
        u32::try_from(image.len() / BLOCK_SECTOR_SIZE).unwrap_or(u32::MAX)
    }

    fn set_lba_byte(&mut self, index: usize, value: u8) {
        let mut bytes = self.lba.to_le_bytes();
        bytes[index] = value;
        self.lba = u32::from_le_bytes(bytes);
    }

    fn run_command(&mut self, command: u8) {
        match command {
            BLOCK_COMMAND_READ => self.read_sector(),
            BLOCK_COMMAND_WRITE => self.write_sector(),
            BLOCK_COMMAND_CLEAR_ERROR => self.error = BLOCK_ERROR_NONE,
            _ => self.error = BLOCK_ERROR_BAD_COMMAND,
        }
    }

    fn read_sector(&mut self) {
        let Some(image) = &self.image else {
            self.error = BLOCK_ERROR_NO_MEDIA;
            return;
        };
        let Some(range) = sector_range(self.lba, image.len()) else {
            self.error = BLOCK_ERROR_OUT_OF_RANGE;
            return;
        };
        self.buffer.copy_from_slice(&image[range]);
        self.index = 0;
        self.error = BLOCK_ERROR_NONE;
    }

    fn write_sector(&mut self) {
        if self.write_protected {
            self.error = BLOCK_ERROR_WRITE_PROTECT;
            return;
        }
        let Some(image) = &mut self.image else {
            self.error = BLOCK_ERROR_NO_MEDIA;
            return;
        };
        let Some(range) = sector_range(self.lba, image.len()) else {
            self.error = BLOCK_ERROR_OUT_OF_RANGE;
            return;
        };
        image[range].copy_from_slice(&self.buffer);
        self.index = 0;
        self.error = BLOCK_ERROR_NONE;
    }
}

fn sector_range(lba: u32, image_len: usize) -> Option<std::ops::Range<usize>> {
    let start = usize::try_from(lba).ok()?.checked_mul(BLOCK_SECTOR_SIZE)?;
    let end = start.checked_add(BLOCK_SECTOR_SIZE)?;
    if end <= image_len {
        Some(start..end)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockRegister {
    Lba0,
    Lba1,
    Lba2,
    Lba3,
    IndexLo,
    IndexHi,
    Data,
    Command,
    Status,
    Error,
    SectorCount0,
    SectorCount1,
    SectorCount2,
    SectorCount3,
}

impl BlockRegister {
    fn sector_count_byte_index(self) -> Option<usize> {
        match self {
            Self::SectorCount0 => Some(0),
            Self::SectorCount1 => Some(1),
            Self::SectorCount2 => Some(2),
            Self::SectorCount3 => Some(3),
            _ => None,
        }
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
    BlockRegister(BlockRegister),
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
    fn cpu_visible_load_honors_romblock_mapping() {
        let mut machine = Machine::new();

        machine.load_cpu_visible(0x00ffee, &[0x34, 0x12]);

        assert_eq!(machine.byte(0x00ffee), 0);
        assert_eq!(machine.byte(0x083fee), 0x34);
        assert_eq!(machine.read(0x00ffee), 0x34);
        assert_eq!(machine.read(0x00ffef), 0x12);
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

    #[test]
    fn block_device_reports_presence_and_capacity() {
        let mut machine = Machine::new();

        assert_eq!(machine.read(BLOCK_STATUS), BLOCK_STATUS_READY);
        assert_eq!(machine.read(BLOCK_SECTOR_COUNT0), 0);

        machine
            .attach_block_device(vec![0; BLOCK_SECTOR_SIZE * 3])
            .expect("block image should attach");

        assert_eq!(
            machine.read(BLOCK_STATUS),
            BLOCK_STATUS_PRESENT | BLOCK_STATUS_READY
        );
        assert_eq!(machine.read(BLOCK_SECTOR_COUNT0), 3);
        assert_eq!(machine.read(BLOCK_SECTOR_COUNT1), 0);
        assert_eq!(machine.read(BLOCK_SECTOR_COUNT2), 0);
        assert_eq!(machine.read(BLOCK_SECTOR_COUNT3), 0);
    }

    #[test]
    fn block_device_reads_selected_sector_into_data_port() {
        let mut image = vec![0; BLOCK_SECTOR_SIZE * 2];
        image[BLOCK_SECTOR_SIZE] = 0x42;
        image[BLOCK_SECTOR_SIZE + 1] = 0x55;
        let mut machine = Machine::new();
        machine
            .attach_block_device(image)
            .expect("block image should attach");

        machine.write(BLOCK_LBA0, 1);
        machine.write(BLOCK_COMMAND, BLOCK_COMMAND_READ);

        assert_eq!(machine.read(BLOCK_ERROR), BLOCK_ERROR_NONE);
        assert_eq!(machine.read(BLOCK_INDEX_LO), 0);
        assert_eq!(machine.read(BLOCK_DATA), 0x42);
        assert_eq!(machine.read(BLOCK_DATA), 0x55);
        assert_eq!(machine.read(BLOCK_INDEX_LO), 2);
    }

    #[test]
    fn block_device_writes_buffer_to_selected_sector() {
        let mut machine = Machine::new();
        machine
            .attach_block_device(vec![0; BLOCK_SECTOR_SIZE * 2])
            .expect("block image should attach");

        machine.write(BLOCK_INDEX_LO, 0);
        machine.write(BLOCK_INDEX_HI, 0);
        machine.write(BLOCK_DATA, 0xaa);
        machine.write(BLOCK_DATA, 0xbb);
        machine.write(BLOCK_LBA0, 1);
        machine.write(BLOCK_COMMAND, BLOCK_COMMAND_WRITE);

        let image = machine
            .block_device_image()
            .expect("block image should remain attached");
        assert_eq!(image[BLOCK_SECTOR_SIZE], 0xaa);
        assert_eq!(image[BLOCK_SECTOR_SIZE + 1], 0xbb);
        assert_eq!(machine.read(BLOCK_ERROR), BLOCK_ERROR_NONE);
    }

    #[test]
    fn block_device_rejects_unaligned_images() {
        let mut machine = Machine::new();

        assert!(machine.attach_block_device(vec![0; 17]).is_err());
        assert_eq!(machine.read(BLOCK_STATUS), BLOCK_STATUS_READY);
    }

    #[test]
    fn block_device_reports_out_of_range_and_write_protect_errors() {
        let mut machine = Machine::new();
        machine
            .attach_write_protected_block_device(vec![0; BLOCK_SECTOR_SIZE])
            .expect("block image should attach");

        machine.write(BLOCK_LBA0, 1);
        machine.write(BLOCK_COMMAND, BLOCK_COMMAND_READ);
        assert_eq!(machine.read(BLOCK_ERROR), BLOCK_ERROR_OUT_OF_RANGE);
        assert_eq!(
            machine.read(BLOCK_STATUS),
            BLOCK_STATUS_PRESENT
                | BLOCK_STATUS_READY
                | BLOCK_STATUS_ERROR
                | BLOCK_STATUS_WRITE_PROTECT
        );

        machine.write(BLOCK_COMMAND, BLOCK_COMMAND_CLEAR_ERROR);
        machine.write(BLOCK_LBA0, 0);
        machine.write(BLOCK_COMMAND, BLOCK_COMMAND_WRITE);
        assert_eq!(machine.read(BLOCK_ERROR), BLOCK_ERROR_WRITE_PROTECT);
    }
}
