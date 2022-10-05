//! ILI9341 Display Driver
//!
//! ### Usage
//!
//! To control the display you need to set up:
//!
//! * Interface for communicating with display ([display-interface-spi crate] for SPI)
//! * Configuration (reset pin, delay, orientation and size) for display
//!
//! ```ignore
//! let iface = SPIInterface::new(spi, dc, cs);
//!
//! let mut display = Ili9341::new(
//!     iface,
//!     reset_gpio,
//!     &mut delay,
//!     Orientation::Landscape,
//!     ili9341::DisplaySize240x320,
//! )
//! .unwrap();
//!
//! display.clear(Rgb565::RED).unwrap()
//! ```
//!
//! [display-interface-spi crate]: https://crates.io/crates/display-interface-spi
use embedded_hal::blocking::delay::DelayMs;
// use embedded_hal::delay::blocking::DelayUs;

use core::iter::once;
use display_interface::DataFormat::{U16BEIter, U8Iter};
use display_interface::WriteOnlyDataCommand;

// mod graphics_core;
use embedded_graphics_core::{
    pixelcolor::{raw::RawU16, Rgb565},
    prelude::*,
    primitives::Rectangle,
};

pub use embedded_hal::spi::MODE_0 as SPI_MODE;

pub use display_interface::DisplayError;
use embedded_graphics_core::draw_target::DrawTarget;

type Result<T = (), E = DisplayError> = core::result::Result<T, E>;

impl<IFACE> OriginDimensions for Ili9342C<IFACE> {
    fn size(&self) -> Size {
        Size::new(self.width() as u32, self.height() as u32)
    }
}

impl<IFACE> DrawTarget for Ili9342C<IFACE>
where
    IFACE: display_interface::WriteOnlyDataCommand,
{
    type Error = display_interface::DisplayError;

    type Color = Rgb565;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if self.bounding_box().contains(point) {
                let x = point.x as u16;
                let y = point.y as u16;

                self.draw_raw_iter(
                    x,
                    y,
                    x,
                    y,
                    core::iter::once(RawU16::from(color).into_inner()),
                )?;
            }
        }
        Ok(())
    }

    fn fill_contiguous<I>(&mut self, area: &Rectangle, colors: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Self::Color>,
    {
        let drawable_area = area.intersection(&self.bounding_box());

        if let Some(drawable_bottom_right) = drawable_area.bottom_right() {
            let x0 = drawable_area.top_left.x as u16;
            let y0 = drawable_area.top_left.y as u16;
            let x1 = drawable_bottom_right.x as u16;
            let y1 = drawable_bottom_right.y as u16;

            if area == &drawable_area {
                // All pixels are on screen
                self.draw_raw_iter(
                    x0,
                    y0,
                    x1,
                    y1,
                    area.points()
                        .zip(colors)
                        .map(|(_, color)| RawU16::from(color).into_inner()),
                )
            } else {
                // Some pixels are on screen
                self.draw_raw_iter(
                    x0,
                    y0,
                    x1,
                    y1,
                    area.points()
                        .zip(colors)
                        .filter(|(point, _)| drawable_area.contains(*point))
                        .map(|(_, color)| RawU16::from(color).into_inner()),
                )
            }
        } else {
            // No pixels are on screen
            Ok(())
        }
    }

    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        self.clear_screen(RawU16::from(color).into_inner())
    }
}

/// Trait that defines display size information
pub trait DisplaySize {
    /// Width in pixels
    const WIDTH: usize;
    /// Height in pixels
    const HEIGHT: usize;
}

/// Generic display size of 240x320 pixels
pub struct DisplaySize320x240;

impl DisplaySize for DisplaySize320x240 {
    const WIDTH: usize = 320;
    const HEIGHT: usize = 240;
}

pub trait Mode {
    fn mode(&self) -> u8;

    fn is_landscape(&self) -> bool;
}

/// The default implementation of the Mode trait from above
/// Should work for most (but not all) boards
#[allow(unused)]
pub enum Orientation {
    Portrait,
    PortraitFlipped,
    Landscape,
    LandscapeFlipped,
}

impl Mode for Orientation {
    fn mode(&self) -> u8 {
        match self {
            Self::Landscape => 0x08,
            Self::Portrait => 0x20 | 0x08,
            Self::LandscapeFlipped => 0x80 | 0x08,
            Self::PortraitFlipped => 0x40 | 0x80 | 0x20 | 0x08,
        }
        // Self::Portrait => 0x40 | 0x08,
        // Self::Landscape => 0x20 | 0x08,
        // Self::PortraitFlipped => 0x80 | 0x08,
        // Self::LandscapeFlipped => 0x40 | 0x80 | 0x20 | 0x08,
        // ili.command(Command::MemoryAccessControl, &[0x40 | 0x20 | 0x08])?;
    }

    fn is_landscape(&self) -> bool {
        match self {
            Self::Landscape | Self::LandscapeFlipped => true,
            Self::Portrait | Self::PortraitFlipped => false,
        }
    }
}

/// Specify state of specific mode of operation
pub enum ModeState {
    On,
    Off,
}

/// There are two method for drawing to the screen:
/// [Ili9341::draw_raw_iter] and [Ili9341::draw_raw_slice]
///
/// In both cases the expected pixel format is rgb565.
///
/// The hardware makes it efficient to draw rectangles on the screen.
///
/// What happens is the following:
///
/// - A drawing window is prepared (with the 2 opposite corner coordinates)
/// - The starting point for drawint is the top left corner of this window
/// - Every pair of bytes received is intepreted as a pixel value in rgb565
/// - As soon as a pixel is received, an internal counter is incremented,
///   and the next word will fill the next pixel (the adjacent on the right, or
///   the first of the next row if the row ended)
#[allow(unused)]
pub struct Ili9342C<IFACE> {
    interface: IFACE,
    width: usize,
    height: usize,
    landscape: bool,
}

impl<IFACE> Ili9342C<IFACE>
where
    IFACE: WriteOnlyDataCommand,
{
    pub fn new<DELAY, SIZE, MODE>(
        interface: IFACE,
        delay: &mut DELAY,
        mode: MODE,
        _display_size: SIZE,
    ) -> Result<Self>
    where
        DELAY: DelayMs<u16>,
        SIZE: DisplaySize,
        MODE: Mode,
    {
        let mut ili = Ili9342C {
            interface,
            width: SIZE::WIDTH,
            height: SIZE::HEIGHT,
            landscape: false,
        };
        ili.command(Command::SoftwareReset, &[])?;
        let _ = delay.delay_ms(10);
        ili.command(Command::ExtC, &[0xff, 0x93, 0x42])?;
        ili.command(Command::PowerControl1, &[0x12, 0x12])?;
        ili.command(Command::PowerControl2, &[0x03])?;
        ili.command(Command::RBGInterface, &[0xe0])?;
        ili.command(Command::InterfaceCtrl, &[0x00, 0x01, 0x01])?;
        // Default is 0x80, 0x20, 0x08
        ili.command(Command::MemoryAccessControl, &[mode.mode()])?;
        //     Orientation::Landscape => mode.mode(),
        //     Orientation::Portrait => mode.mode(),
        //     Orientation::LandscapeFlipped => mode.mode(),
        //     Orientation::PortraitFlipped => mode.mode(),
        // };
        // ili.command(Command::MemoryAccessControl, &[0x40 | 0x20 | 0x08])?;
        ili.command(Command::PixelFormatSet, &[0x55])?;
        ili.command(Command::DisplayFunctionControl, &[0x08, 0x82, 0x27])?;
        ili.command(
            Command::GammaControlPos1,
            &[
                0x00, 0x0c, 0x11, 0x04, 0x11, 0x08, 0x37, 0x89, 0x4c, 0x06, 0x0c, 0x0a, 0x2e, 0x34,
                0x0f,
            ],
        )?;
        ili.command(
            Command::GammaControlNeg1,
            &[
                0x00, 0x0b, 0x11, 0x05, 0x13, 0x09, 0x33, 0x67, 0x48, 0x07, 0x0e, 0x0b, 0x2e, 0x33,
                0x0f,
            ],
        )?;
        ili.sleep_mode(ModeState::Off)?;
        let _ = delay.delay_ms(120);
        ili.display_mode(ModeState::On)?;
        ili.command(Command::InvertOn, &[])?;

        // Wait 5ms after Sleep Out before sending commands
        let _ = delay.delay_ms(5);

        Ok(ili)
    }
}

impl<IFACE> Ili9342C<IFACE>
where
    IFACE: WriteOnlyDataCommand,
{
    fn command(&mut self, cmd: Command, args: &[u8]) -> Result {
        self.interface.send_commands(U8Iter(&mut once(cmd as u8)))?;
        self.interface.send_data(U8Iter(&mut args.iter().cloned()))
    }

    fn write_iter<I: IntoIterator<Item = u16>>(&mut self, data: I) -> Result {
        self.command(Command::MemoryWrite, &[])?;
        self.interface.send_data(U16BEIter(&mut data.into_iter()))
    }

    fn set_window(&mut self, x0: u16, y0: u16, x1: u16, y1: u16) -> Result {
        self.command(
            Command::ColumnAddressSet,
            &[
                (x0 >> 8) as u8,
                (x0 & 0xff) as u8,
                (x1 >> 8) as u8,
                (x1 & 0xff) as u8,
            ],
        )?;
        self.command(
            Command::PageAddressSet,
            &[
                (y0 >> 8) as u8,
                (y0 & 0xff) as u8,
                (y1 >> 8) as u8,
                (y1 & 0xff) as u8,
            ],
        )
    }

    // /// Configures the screen for hardware-accelerated vertical scrolling.
    // pub fn configure_vertical_scroll(
    //     &mut self,
    //     fixed_top_lines: u16,
    //     fixed_bottom_lines: u16,
    // ) -> Result<Scroller> {
    //     let height = if self.landscape {
    //         self.width
    //     } else {
    //         self.height
    //     } as u16;
    //     let scroll_lines = height as u16 - fixed_top_lines - fixed_bottom_lines;

    //     self.command(
    //         Command::VerticalScrollDefine,
    //         &[
    //             (fixed_top_lines >> 8) as u8,
    //             (fixed_top_lines & 0xff) as u8,
    //             (scroll_lines >> 8) as u8,
    //             (scroll_lines & 0xff) as u8,
    //             (fixed_bottom_lines >> 8) as u8,
    //             (fixed_bottom_lines & 0xff) as u8,
    //         ],
    //     )?;

    //     Ok(Scroller::new(fixed_top_lines, fixed_bottom_lines, height))
    // }

    // pub fn scroll_vertically(&mut self, scroller: &mut Scroller, num_lines: u16) -> Result {
    //     scroller.top_offset += num_lines;
    //     if scroller.top_offset > (scroller.height - scroller.fixed_bottom_lines) {
    //         scroller.top_offset = scroller.fixed_top_lines
    //             + (scroller.top_offset + scroller.fixed_bottom_lines - scroller.height)
    //     }

    //     self.command(
    //         Command::VerticalScrollAddr,
    //         &[
    //             (scroller.top_offset >> 8) as u8,
    //             (scroller.top_offset & 0xff) as u8,
    //         ],
    //     )
    // }

    /// Draw a rectangle on the screen, represented by top-left corner (x0, y0)
    /// and bottom-right corner (x1, y1).
    ///
    /// The border is included.
    ///
    /// This method accepts an iterator of rgb565 pixel values.
    ///
    /// The iterator is useful to avoid wasting memory by holding a buffer for
    /// the whole screen when it is not necessary.
    pub fn draw_raw_iter<I: IntoIterator<Item = u16>>(
        &mut self,
        x0: u16,
        y0: u16,
        x1: u16,
        y1: u16,
        data: I,
    ) -> Result {
        self.set_window(x0, y0, x1, y1)?;
        self.write_iter(data)
    }

    /// Change the orientation of the screen
    pub fn set_orientation<MODE>(&mut self, mode: MODE) -> Result
    where
        MODE: Mode,
    {
        self.command(Command::MemoryAccessControl, &[mode.mode()])?;

        if self.landscape ^ mode.is_landscape() {
            core::mem::swap(&mut self.height, &mut self.width);
        }
        self.landscape = mode.is_landscape();
        Ok(())
    }

    /// Fill entire screen with specfied color u16 value
    pub fn clear_screen(&mut self, color: u16) -> Result {
        let color = core::iter::repeat(color).take(self.width * self.height);
        self.draw_raw_iter(0, 0, self.width as u16, self.height as u16, color)
    }

    /// Control the screen sleep mode:
    pub fn sleep_mode(&mut self, mode: ModeState) -> Result {
        match mode {
            ModeState::On => self.command(Command::SleepModeOn, &[]),
            ModeState::Off => self.command(Command::SleepModeOff, &[]),
        }
    }

    /// Control the screen display mode
    pub fn display_mode(&mut self, mode: ModeState) -> Result {
        match mode {
            ModeState::On => self.command(Command::DisplayOn, &[]),
            ModeState::Off => self.command(Command::DisplayOff, &[]),
        }
    }
}

impl<IFACE> Ili9342C<IFACE> {
    /// Get the current screen width. It can change based on the current orientation
    pub fn width(&self) -> usize {
        self.width
    }

    /// Get the current screen heighth. It can change based on the current orientation
    pub fn height(&self) -> usize {
        self.height
    }
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
enum Command {
    SoftwareReset = 0x01,
    SleepModeOn = 0x10,
    SleepModeOff = 0x11,
    InvertOff = 0x20,
    InvertOn = 0x21,
    DisplayOff = 0x28,
    DisplayOn = 0x29,
    ColumnAddressSet = 0x2a,
    PageAddressSet = 0x2b,
    MemoryWrite = 0x2c,
    PixelFormatSet = 0x3a,
    VerticalScrollDefine = 0x33,
    MemoryAccessControl = 0x36,
    VerticalScrollAddr = 0x37,
    IdleModeOff = 0x38,
    IdleModeOn = 0x39,
    SetBrightness = 0x51,
    ContentAdaptiveBrightness = 0x55,
    RBGInterface = 0xb0,
    FrameControl = 0xb1,
    IdleModeFrameRate = 0xb2,
    DisplayFunctionControl = 0xb6,
    PowerControl1 = 0xc0,
    PowerControl2 = 0xc1,
    ExtC = 0xc8,
    GammaControlPos1 = 0xe0,
    GammaControlNeg1 = 0xe1,
    InterfaceCtrl = 0xf6,
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
