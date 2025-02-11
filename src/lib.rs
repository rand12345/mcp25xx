//! `no_std` library for the MCP2510, MCP2515 and MCP25625 CAN controller chips.
//!
//! API is implemented in terms of of the [`embedded_hal`] and [`embedded_can`] traits.
//!
//! Activating the `mcp2515` or `mcp25625` feature will enable
//! additional registers and instructions the MCP2510 does not support.
//!
//! # Example
//!
//! ```
//! use embedded_can::blocking::Can as _;
//! use embedded_can::{Can, Frame, StandardId};
//! use mcp25xx::bitrates::clock_16mhz::CNF_500K_BPS;
//! # use mcp25xx::doctesthelper::{NoOpCS, NoOpSPI};
//! use mcp25xx::registers::{OperationMode, RXB0CTRL, RXM};
//! use mcp25xx::{CanFrame, Config, MCP25xx};
//!
//! # let spi = NoOpSPI;
//! # let cs = NoOpCS;
//! #
//! // spi, cs and timer are structs implementing their respective embedded_hal traits.
//!
//! let mut mcp25xx = MCP25xx { spi, cs };
//!
//! let config = Config::default()
//!     .mode(OperationMode::NormalOperation)
//!     .bitrate(CNF_500K_BPS)
//!     .receive_buffer_0(RXB0CTRL::default().with_rxm(RXM::ReceiveAny));
//!
//! mcp25xx.apply_config(&config).unwrap();
//!
//! // Send a frame
//! let can_id = StandardId::new(123).unwrap();
//! let data = [1, 2, 3, 4, 5, 6, 7, 8];
//! let frame = CanFrame::new(can_id, &data).unwrap();
//! mcp25xx.try_write(&frame).unwrap();
//!
//! // Receive a frame
//! if let Ok(frame) = mcp25xx.try_receive() {
//!     let _can_id = frame.id();
//!     let _data = frame.data();
//! }
//! ```

#![no_std]
#![cfg_attr(doc, feature(doc_cfg))]
use core::fmt::Debug;

pub use embedded_can;
use embedded_can::Frame;

pub use config::Config;
pub use frame::CanFrame;
pub use idheader::IdHeader;
pub use spi_trait::SpiWithCs;

use crate::registers::*;

/// Preconfigured CNF registers for 8, 16 and 20 Mhz oscillators
pub mod bitrates;
/// Register bitfields
pub mod registers;

mod config;
mod frame;
mod idheader;
mod spi_trait;

/// Either a MCP2510, MCP2515 or MCP25625 CAN controller
///
/// ## Note about MCP2515 and MCP25625
/// These chip revisions offer more efficient commands which the MCP2510 does not support.
/// You can opt in to using these by activating the `mcp2515` or `mcp25625` feature of this crate.
pub struct MCP25xx<SPI, CS> {
    pub spi: SPI,
    pub cs: CS,
}

impl<SPI, CS> MCP25xx<SPI, CS>
where
    Self: SpiWithCs,
{
    /// Performs the following steps:
    /// * resets the CAN Controller (this resets all registers and puts it into configuration mode)
    /// * applies configuration
    /// * applies selected operation mode
    ///
    /// ## Note about Masks
    /// The default state of the mask registers is all zeros, which means, filters get ignored.
    /// You should give values for both mask registers even if you only intend to use one receive buffer.
    ///
    /// ```
    /// # use mcp25xx::doctesthelper::get_mcp25xx;
    /// # use mcp25xx::{AcceptanceFilter, Config, MCP25xx};
    /// # use mcp25xx::registers::OperationMode;
    /// # use mcp25xx::bitrates::clock_16mhz::CNF_500K_BPS;
    /// # use embedded_can::StandardId;
    ///
    /// let mut mcp25xx: MCP25xx<_, _> = get_mcp25xx();
    ///
    /// let can_id = StandardId::new(123).unwrap();
    /// let filters = [
    ///     (AcceptanceFilter::Filter0, can_id.into()),
    ///     (AcceptanceFilter::Mask0, StandardId::MAX.into()),
    ///     (AcceptanceFilter::Mask1, StandardId::MAX.into()),
    /// ];
    ///
    /// let config = Config::default()
    ///     .mode(OperationMode::NormalOperation)
    ///     .bitrate(CNF_500K_BPS)
    ///     .filters(&filters);
    /// mcp25xx.apply_config(&config).unwrap();
    /// ```
    pub fn apply_config(&mut self, config: &Config<'_>) -> Result<(), <Self as SpiWithCs>::Error> {
        self.reset()?;
        self.set_bitrate(config.cnf)?;
        self.write_register(config.rxb0ctrl)?;
        self.write_register(config.rxb1ctrl)?;
        for &(filter, id_header) in config.filters {
            self.set_filter(filter, id_header)?;
        }
        self.write_register(config.canctrl)
    }

    /// Set the controller to NormalOperation, Sleep, Loopback, ListenOnly or Configuration
    pub fn set_mode(&mut self, mode: OperationMode) -> Result<(), <Self as SpiWithCs>::Error> {
        let reg = CANCTRL::new().with_reqop(mode);
        self.modify_register(reg, 0b11100000)
    }

    /// Set clock settings
    ///
    /// See [`bitrates`] for preconfigured settings for different oscillator frequencies.
    ///
    /// ## Note:
    /// The controller needs to be in Configuration Mode for this
    pub fn set_bitrate(&mut self, cnf: CNF) -> Result<(), <Self as SpiWithCs>::Error> {
        self.write_registers(CNF3::ADDRESS, &cnf.into_bytes())
    }

    /// Set individual receive buffer filters or masks
    ///
    /// ## Note:
    /// The controller needs to be in Configuration Mode for this
    ///
    /// ```
    /// # use mcp25xx::doctesthelper::get_mcp25xx;
    /// use embedded_can::{StandardId, ExtendedId};
    /// use mcp25xx::{MCP25xx, IdHeader, AcceptanceFilter::*};
    ///
    /// let mut mcp25xx: MCP25xx<_, _> = get_mcp25xx();
    ///
    /// let std_id = StandardId::new(1234).unwrap();
    /// let ext_id = ExtendedId::new(4321).unwrap();
    ///
    /// mcp25xx.set_filter(Mask0, IdHeader::from(StandardId::MAX));
    /// mcp25xx.set_filter(Filter0, IdHeader::from(std_id)).unwrap();
    ///
    /// mcp25xx.set_filter(Filter2, IdHeader::from(ext_id)).unwrap();
    /// #[cfg(any(feature = "mcp2515", feature = "mcp25625"))]
    /// mcp25xx.set_filter(Filter3, IdHeader::with_two_data_bytes(std_id, [4, 5])).unwrap();
    ///
    /// ```
    pub fn set_filter(
        &mut self,
        filter: AcceptanceFilter,
        id: IdHeader,
    ) -> Result<(), <Self as SpiWithCs>::Error> {
        self.write_registers(filter as u8, &id.into_bytes())
    }

    /// Read status flags
    pub fn read_status(&mut self) -> Result<ReadStatusResponse, <Self as SpiWithCs>::Error> {
        self.set_cs_low();
        self.spi_write(&[Instruction::ReadStatus as u8])?;
        let mut buf = [0];
        self.spi_transfer(&mut buf)?;
        self.set_cs_high();
        Ok(ReadStatusResponse::from_bytes(buf))
    }

    /// Reset internal registers to the default state. Sets Configuration mode.
    pub fn reset(&mut self) -> Result<(), <Self as SpiWithCs>::Error> {
        self.set_cs_low();
        self.spi_write(&[Instruction::Reset as u8])?;
        self.set_cs_high();
        Ok(())
    }

    /// Read receive buffer status flags
    #[cfg(any(feature = "mcp2515", feature = "mcp25625"))]
    #[cfg_attr(doc, doc(cfg(any(feature = "mcp2515", feature = "mcp25625"))))]
    pub fn rx_status(&mut self) -> Result<RxStatusResponse, <Self as SpiWithCs>::Error> {
        self.set_cs_low();
        self.spi_write(&[Instruction::RxStatus as u8])?;
        let mut buf = [0];
        self.spi_transfer(&mut buf)?;
        self.set_cs_high();
        Ok(RxStatusResponse::from_bytes(buf))
    }
}

impl<SPI, CS> embedded_can::Can for MCP25xx<SPI, CS>
where
    Self: SpiWithCs,
{
    type Frame = crate::frame::CanFrame;
    type Error = <Self as SpiWithCs>::Error;

    fn try_transmit(
        &mut self,
        frame: &Self::Frame,
    ) -> nb::Result<Option<Self::Frame>, <Self as SpiWithCs>::Error> {
        let status = self.read_status()?;
        let mut buf_idx = TxBuffer::TXB0;
        if status.txreq0() {
            buf_idx = TxBuffer::TXB1;
            if status.txreq1() {
                buf_idx = TxBuffer::TXB2;
                if status.txreq2() {
                    // TODO replace a pending lower priority frame
                    return Err(nb::Error::WouldBlock);
                }
            }
        }

        self.load_tx_buffer(buf_idx, frame)?;
        self.request_to_send(buf_idx)?;
        Ok(None)
    }

    fn try_receive(&mut self) -> nb::Result<Self::Frame, <Self as SpiWithCs>::Error> {
        // TODO look at https://www.microchip.com/forums/tm.aspx?m=620741
        let status = self.read_status()?;
        if status.rx0if() {
            Ok(self.read_rx_buffer(RxBuffer::RXB0)?)
        } else if status.rx1if() {
            Ok(self.read_rx_buffer(RxBuffer::RXB1)?)
        } else {
            Err(nb::Error::WouldBlock)
        }
    }
}

impl<SPI, CS> embedded_can::blocking::Default for MCP25xx<SPI, CS> where Self: SpiWithCs {}

impl<SPI, CS> MCP25xx<SPI, CS>
where
    Self: SpiWithCs,
{
    /// Read a single register
    pub fn read_register<R: Register>(&mut self) -> Result<R, <Self as SpiWithCs>::Error> {
        self.set_cs_low();
        self.spi_write(&[Instruction::Read as u8, R::ADDRESS])?;
        let mut reg = [0];
        self.spi_transfer(&mut reg)?;
        self.set_cs_high();
        Ok(reg[0].into())
    }

    /// Write a single register
    pub fn write_register<R: Register + Into<u8>>(
        &mut self,
        reg: R,
    ) -> Result<(), <Self as SpiWithCs>::Error> {
        self.set_cs_low();
        self.spi_write(&[Instruction::Write as u8, R::ADDRESS, reg.into()])?;
        self.set_cs_high();
        Ok(())
    }

    /// Modify a single register
    ///
    /// Only registers implementing [`Modify`] support the `Modify` Instruction
    pub fn modify_register<R: Register + Modify + Into<u8>>(
        &mut self,
        reg: R,
        mask: u8,
    ) -> Result<(), <Self as SpiWithCs>::Error> {
        self.set_cs_low();
        self.spi_write(&[Instruction::BitModify as u8, R::ADDRESS, mask, reg.into()])?;
        self.set_cs_high();
        Ok(())
    }

    /// Read multiple consecutive registers
    pub fn read_registers(
        &mut self,
        start_address: u8,
        buf: &mut [u8],
    ) -> Result<(), <Self as SpiWithCs>::Error> {
        self.set_cs_low();
        self.spi_write(&[Instruction::Read as u8, start_address])?;
        self.spi_transfer(buf)?;
        self.set_cs_high();
        Ok(())
    }

    /// Write multiple consecutive registers
    pub fn write_registers(
        &mut self,
        start_address: u8,
        data: &[u8],
    ) -> Result<(), <Self as SpiWithCs>::Error> {
        self.set_cs_low();
        self.spi_write(&[Instruction::Write as u8, start_address])?;
        self.spi_write(data)?;
        self.set_cs_high();
        Ok(())
    }

    /// Request the selected transmit buffer to send a CAN frame
    pub fn request_to_send(&mut self, buf_idx: TxBuffer) -> Result<(), <Self as SpiWithCs>::Error> {
        self.set_cs_low();
        self.spi_write(&[Instruction::Rts as u8 | (1 << buf_idx as u8)])?;
        self.set_cs_high();
        Ok(())
    }

    /// Setup the selected transmit buffer with CAN frame data
    #[cfg(any(feature = "mcp2515", feature = "mcp25625"))]
    pub fn load_tx_buffer(
        &mut self,
        buf_idx: TxBuffer,
        frame: &CanFrame,
    ) -> Result<(), <Self as SpiWithCs>::Error> {
        let data = &frame.as_bytes()[0..5 + frame.dlc()];

        self.set_cs_low();
        self.spi_write(&[Instruction::LoadTxBuffer as u8 | (buf_idx as u8 * 2)])?;
        self.spi_write(data)?;
        self.set_cs_high();
        Ok(())
    }

    /// Setup the selected transmit buffer with CAN frame data
    #[cfg(not(any(feature = "mcp2515", feature = "mcp25625")))]
    #[inline]
    pub fn load_tx_buffer(
        &mut self,
        buf_idx: TxBuffer,
        frame: &CanFrame,
    ) -> Result<(), <Self as SpiWithCs>::Error> {
        let data = &frame.as_bytes()[0..5 + frame.dlc()];
        self.write_registers(0x31 + 0x10 * buf_idx as u8, data)
    }

    /// Read CAN frame data from the selected receive buffer
    pub fn read_rx_buffer(
        &mut self,
        buf_idx: RxBuffer,
    ) -> Result<crate::frame::CanFrame, <Self as SpiWithCs>::Error> {
        // gets a view into the first 5 bytes of Frame
        fn id_bytes(frame: &mut crate::frame::CanFrame) -> &mut [u8; 5] {
            // SAFETY:
            // Frame is [repr(C)] without any padding bytes
            // All bit patterns are valid
            unsafe { &mut *(frame as *mut crate::frame::CanFrame as *mut [u8; 5]) }
        }

        let mut frame = crate::frame::CanFrame::default();

        self.set_cs_low();

        self.send_read_rx_instruction(buf_idx)?;
        self.spi_transfer(id_bytes(&mut frame))?;
        let mut dlc = frame.dlc();
        if dlc > 8 {
            dlc = 8;
            frame.dlc.set_dlc(8);
        }
        self.spi_transfer(&mut frame.data[0..dlc])?;

        self.set_cs_high();

        #[cfg(not(any(feature = "mcp2515", feature = "mcp25625")))]
        // need to manually reset the interrupt flag bit if Instruction::ReadRxBuffer is not available
        self.modify_register(CANINTF::new(), 1 << buf_idx as u8)?;
        Ok(frame)
    }

    #[cfg(any(feature = "mcp2515", feature = "mcp25625"))]
    fn send_read_rx_instruction(
        &mut self,
        buf_idx: RxBuffer,
    ) -> Result<(), <Self as SpiWithCs>::Error> {
        self.spi_write(&[Instruction::ReadRxBuffer as u8 | (buf_idx as u8 * 2)])
    }

    #[cfg(not(any(feature = "mcp2515", feature = "mcp25625")))]
    fn send_read_rx_instruction(
        &mut self,
        buf_idx: RxBuffer,
    ) -> Result<(), <Self as SpiWithCs>::Error> {
        self.spi_write(&[Instruction::Read as u8, 0x61 + 0x10 * buf_idx as u8])
    }
}

/// Filters and Masks of the two receive buffers
#[derive(Copy, Clone, Debug)]
pub enum AcceptanceFilter {
    /// Associated with Receive Buffer 0
    Filter0 = 0x00,
    /// Associated with Receive Buffer 0
    Filter1 = 0x04,
    /// Associated with Receive Buffer 1
    Filter2 = 0x08,
    /// Associated with Receive Buffer 1
    Filter3 = 0x10,
    /// Associated with Receive Buffer 1
    Filter4 = 0x14,
    /// Associated with Receive Buffer 1
    Filter5 = 0x18,
    /// Associated with Receive Buffer 0
    Mask0 = 0x20,
    /// Associated with Receive Buffer 1
    Mask1 = 0x24,
}

/// Transmit buffer
#[derive(Copy, Clone, Debug)]
pub enum TxBuffer {
    /// Transmit buffer 0
    TXB0 = 0,
    /// Transmit buffer 1
    TXB1 = 1,
    /// Transmit buffer 2
    TXB2 = 2,
}

/// Receive buffer
#[derive(Copy, Clone, Debug)]
pub enum RxBuffer {
    /// Receive Buffer 0
    RXB0 = 0,
    /// Receive Buffer 1
    RXB1 = 1,
}

/// Instruction supported by the CAN controller
#[derive(Copy, Clone, Debug)]
#[repr(u8)]
pub enum Instruction {
    /// Resets internal registers to the default state, sets Configuration mode.
    Reset = 0b1100_0000,
    /// Reads data from the register beginning at the selected address.
    Read = 0b0000_0011,
    /// Writes data to the register beginning at the selected address.
    Write = 0b0000_0010,
    /// Instructs the controller to begin the message transmission sequence for
    /// any of the transmit buffers specified in `0b1000_0nnn`.
    Rts = 0b1000_0000,
    /// Quick polling command that reads several Status bits for transmit and receive functions.
    ReadStatus = 0b1010_0000,
    /// Allows the user to set or clear individual bits in a particular register.
    ///
    /// Note: Not all registers can be bit modified with this command.
    /// Executing this command on registers that are not bit modifiable will force the mask to FFh.
    ///
    /// Registers that can be modified with this command implement [`Modify`].
    BitModify = 0b0000_0101,

    #[cfg(any(feature = "mcp2515", feature = "mcp25625"))]
    #[cfg_attr(doc, doc(cfg(any(feature = "mcp2515", feature = "mcp25625"))))]
    /// Quick polling command that indicates a filter match and message type
    /// (standard, extended and/or remote) of the received message.
    RxStatus = 0b1011_0000,
    #[cfg(any(feature = "mcp2515", feature = "mcp25625"))]
    #[cfg_attr(doc, doc(cfg(any(feature = "mcp2515", feature = "mcp25625"))))]
    /// When reading a receive buffer, reduces the overhead of a normal `Read`
    /// command by placing the Address Pointer at one of four locations, as
    /// indicated by ‘nm’ in `0b1001_0nm0`.
    ///
    /// Note: The associated RX flag bit (`rxNif` bits in the [`CANINTF`] register) will be cleared after bringing CS high.
    ReadRxBuffer = 0b1001_0000,
    #[cfg(any(feature = "mcp2515", feature = "mcp25625"))]
    #[cfg_attr(doc, doc(cfg(any(feature = "mcp2515", feature = "mcp25625"))))]
    /// When loading a transmit buffer, reduces the overhead of a normal `Write`
    /// command by placing the Address Pointer at one of six locations, as
    /// indicated by ‘abc’ in `0b0100_0abc`.
    LoadTxBuffer = 0b0100_0000,
}

#[doc(hidden)]
// FIXME: #[cfg(doctest)] once https://github.com/rust-lang/rust/issues/67295 is fixed.
pub mod doctesthelper;
