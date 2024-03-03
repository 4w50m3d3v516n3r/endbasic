// EndBASIC
// Copyright 2024 Julio Merino
//
// Licensed under the Apache License, Version 2.0 (the "License"); you may not
// use this file except in compliance with the License.  You may obtain a copy
// of the License at:
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
// WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.  See the
// License for the specific language governing permissions and limitations
// under the License.

/***************************************************************************************************
* | file        :    LCD_Driver.c
* | version     :    V1.0
* | date        :    2017-10-16
* | function    :    On the ST7735S chip driver and clear screen, drawing lines, drawing, writing
                     and other functions to achieve
***************************************************************************************************/

//! Console driver for the ST7735S LCD.

use crate::gpio::gpio_error_to_io_error;
use crate::lcd::{to_xy_size, BufferedLcd, Lcd, LcdSize, LcdXY};
use async_channel::Sender;
use async_trait::async_trait;
use endbasic_core::exec::Signal;
use endbasic_std::console::graphics::InputOps;
use endbasic_std::console::{
    CharsXY, ClearType, Console, GraphicsConsole, Key, PixelsXY, SizeInPixels, RGB,
};
use endbasic_terminal::TerminalConsole;
use rppal::gpio::{Gpio, Level, OutputPin};
use rppal::spi::{self, Bus, SlaveSelect, Spi};
use std::io;
use std::time::Duration;

/// Converts an SPI error to an IO error.
fn spi_error_to_io_error(e: spi::Error) -> io::Error {
    match e {
        spi::Error::Io(e) => e,
        e => io::Error::new(io::ErrorKind::InvalidInput, e.to_string()),
    }
}

/// Input handler for the ST7735S console.
///
/// This relies on the usual terminal console in raw mode to gather keyboard input but also adds
/// support for the physical buttons that come along with the display.
struct ST7735SInput {
    terminal: TerminalConsole,
}

impl ST7735SInput {
    fn new(signals_tx: Sender<Signal>) -> io::Result<Self> {
        let terminal = TerminalConsole::from_stdio(signals_tx)?;

        // TODO(jmmv): Set up and handle the physical buttons.

        Ok(Self { terminal })
    }
}

#[async_trait(?Send)]
impl InputOps for ST7735SInput {
    async fn poll_key(&mut self) -> io::Result<Option<Key>> {
        self.terminal.poll_key().await
    }

    async fn read_key(&mut self) -> io::Result<Key> {
        self.terminal.read_key().await
    }
}

/// LCD handler for the ST7735S console.
struct ST7735SLcd {
    spi: Spi,

    lcd_rst: OutputPin,
    lcd_dc: OutputPin,
    lcd_bl: OutputPin,

    size_pixels: LcdSize,
}

impl ST7735SLcd {
    /// Initializes the LCD.
    pub fn new(gpio: &mut Gpio) -> io::Result<Self> {
        let mut lcd_cs = gpio.get(8).map_err(gpio_error_to_io_error)?.into_output();
        let lcd_rst = gpio.get(27).map_err(gpio_error_to_io_error)?.into_output();
        let lcd_dc = gpio.get(25).map_err(gpio_error_to_io_error)?.into_output();
        let mut lcd_bl = gpio.get(24).map_err(gpio_error_to_io_error)?.into_output();

        lcd_cs.write(Level::High);
        lcd_bl.write(Level::High);

        let spi = Spi::new(Bus::Spi0, SlaveSelect::Ss0, 9000000, rppal::spi::Mode::Mode0)
            .map_err(spi_error_to_io_error)?;
        spi.set_ss_polarity(spi::Polarity::ActiveLow).map_err(spi_error_to_io_error)?;

        let size_pixels = LcdSize { width: 128, height: 128 };

        let mut device = Self { lcd_rst, lcd_dc, lcd_bl, spi, size_pixels };

        device.lcd_init()?;

        Ok(device)
    }

    /// Writes arbitrary data to the SPI bus.
    ///
    /// The input data is chunked to respect the maximum write size accepted by the SPI bus.
    fn lcd_write(&mut self, data: &[u8]) -> io::Result<()> {
        // TODO(jmmv): Read /sys/module/spidev/parameters/bufsiz and leverage larger writes if.
        // possible.
        for chunk in data.chunks(4096) {
            let mut i = 0;
            loop {
                let n = self.spi.write(&chunk[i..]).map_err(spi_error_to_io_error)?;
                if n == 0 {
                    break;
                }
                i += n;
            }
        }
        Ok(())
    }

    /// Selects the registers to affect by the next data write.
    fn lcd_write_reg(&mut self, regs: &[u8]) -> io::Result<()> {
        self.lcd_dc.write(Level::Low);
        self.lcd_write(regs)
    }

    /// Writes data to the device.  A register should have been selected before.
    fn lcd_write_data(&mut self, data: &[u8]) -> io::Result<()> {
        self.lcd_dc.write(Level::High);
        self.lcd_write(data)
    }

    /// Resets the LCD.
    fn lcd_reset(&mut self) {
        self.lcd_rst.write(Level::High);
        std::thread::sleep(Duration::from_millis(100));
        self.lcd_rst.write(Level::Low);
        std::thread::sleep(Duration::from_millis(100));
        self.lcd_rst.write(Level::High);
        std::thread::sleep(Duration::from_millis(100));
    }

    /// Sets up the LCD registers.
    fn lcd_init_reg(&mut self) -> io::Result<()> {
        // ST7735R Frame Rate.
        self.lcd_write_reg(&[0xb1])?;
        self.lcd_write_data(&[0x01, 0x2c, 0x2d])?;

        self.lcd_write_reg(&[0xb2])?;
        self.lcd_write_data(&[0x01, 0x2c, 0x2d])?;

        self.lcd_write_reg(&[0xb3])?;
        self.lcd_write_data(&[0x01, 0x2c, 0x2d, 0x01, 0x2c, 0x2d])?;

        // Column inversion.
        self.lcd_write_reg(&[0xb4])?;
        self.lcd_write_data(&[0x07])?;

        // ST7735R Power Sequence.
        self.lcd_write_reg(&[0xc0])?;
        self.lcd_write_data(&[0xa2, 0x02, 0x84])?;
        self.lcd_write_reg(&[0xc1])?;
        self.lcd_write_data(&[0xc5])?;

        self.lcd_write_reg(&[0xc2])?;
        self.lcd_write_data(&[0x0a, 0x00])?;

        self.lcd_write_reg(&[0xc3])?;
        self.lcd_write_data(&[0x8a, 0x2a])?;
        self.lcd_write_reg(&[0xc4])?;
        self.lcd_write_data(&[0x8a, 0xee])?;

        // VCOM.
        self.lcd_write_reg(&[0xc5])?;
        self.lcd_write_data(&[0x0e])?;

        // ST7735R Gamma Sequence.
        self.lcd_write_reg(&[0xe0])?;
        self.lcd_write_data(&[
            0x0f, 0x1a, 0x0f, 0x18, 0x2f, 0x28, 0x20, 0x22, 0x1f, 0x1b, 0x23, 0x37, 0x00, 0x07,
            0x02, 0x10,
        ])?;

        self.lcd_write_reg(&[0xe1])?;
        self.lcd_write_data(&[
            0x0f, 0x1b, 0x0f, 0x17, 0x33, 0x2c, 0x29, 0x2e, 0x30, 0x30, 0x39, 0x3f, 0x00, 0x07,
            0x03, 0x10,
        ])?;

        // Enable test command.
        self.lcd_write_reg(&[0xf0])?;
        self.lcd_write_data(&[0x01])?;

        // Disable ram power save mode.
        self.lcd_write_reg(&[0xf6])?;
        self.lcd_write_data(&[0x00])?;

        // 65k mode.
        self.lcd_write_reg(&[0x3a])?;
        self.lcd_write_data(&[0x05])?;

        Ok(())
    }

    /// Initializes the LCD scan direction and pixel color encoding.
    fn lcd_set_gram_scan_way(&mut self) -> io::Result<()> {
        self.lcd_write_reg(&[0x36])?; // MX, MY, RGB mode.
        let scan_dir = 0x40 | 0x20; // X, Y.
        let rgb_mode = 0x08; // RGB for 1.44in display.
        self.lcd_write_data(&[scan_dir | rgb_mode])?;
        Ok(())
    }

    /// Initializes the LCD.
    fn lcd_init(&mut self) -> io::Result<()> {
        self.lcd_bl.write(Level::High);

        self.lcd_reset();
        self.lcd_init_reg()?;

        self.lcd_set_gram_scan_way()?;
        std::thread::sleep(Duration::from_millis(200));

        self.lcd_write_reg(&[0x11])?;
        std::thread::sleep(Duration::from_millis(200));

        // Turn display on.
        self.lcd_write_reg(&[0x29])?;

        Ok(())
    }

    /// Configures the LCD so that the next write, which carries pixel data, affects the specified
    /// region.
    fn lcd_set_window(&mut self, xy: LcdXY, size: LcdSize) -> io::Result<()> {
        let adjust_x = 1;
        let adjust_y = 2;

        let x1 = ((xy.x & 0xff) + adjust_x) as u8;
        let x2 = (((xy.x + size.width) + adjust_x - 1) & 0xff) as u8;
        let y1 = ((xy.y & 0xff) + adjust_y) as u8;
        let y2 = (((xy.y + size.height) + adjust_y - 1) & 0xff) as u8;

        self.lcd_write_reg(&[0x2a])?;
        self.lcd_write_data(&[0x00, x1, 0x00, x2])?;

        self.lcd_write_reg(&[0x2b])?;
        self.lcd_write_data(&[0x00, y1, 0x00, y2])?;

        self.lcd_write_reg(&[0x2c])?;

        Ok(())
    }
}

impl Drop for ST7735SLcd {
    fn drop(&mut self) {
        self.lcd_bl.write(Level::Low);
    }
}

impl Lcd for ST7735SLcd {
    type Pixel = [u8; 2];

    fn info(&self) -> (LcdSize, usize) {
        (self.size_pixels, 2)
    }

    fn encode(&self, rgb: RGB) -> Self::Pixel {
        let rgb = (u16::from(rgb.0), u16::from(rgb.1), u16::from(rgb.2));

        // RGB565 format.
        let pixel: u16 = ((rgb.0 >> 3) << 11) | ((rgb.1 >> 2) << 5) | (rgb.2 >> 3);

        let high = (pixel >> 8) as u8;
        let low = (pixel & 0xff) as u8;
        [high, low]
    }

    fn set_data(&mut self, x1y1: LcdXY, x2y2: LcdXY, data: &[u8]) -> io::Result<()> {
        let (xy, size) = to_xy_size(x1y1, x2y2);
        self.lcd_set_window(xy, size)?;
        self.lcd_write_data(data)
    }
}

/// Console implementation using a ST7735S LCD.
pub struct ST7735SConsole {
    /// GPIO controller used for the LCD and the input buttons.  Must be kept alive for as long as
    /// `inner` is.
    _gpio: Gpio,

    /// The graphical console itself.  We wrap it in a struct to prevent leaking all auxiliary types
    /// outside of this crate.
    inner: GraphicsConsole<ST7735SInput, BufferedLcd<ST7735SLcd>>,
}

#[async_trait(?Send)]
impl Console for ST7735SConsole {
    fn clear(&mut self, how: ClearType) -> io::Result<()> {
        self.inner.clear(how)
    }

    fn color(&self) -> (Option<u8>, Option<u8>) {
        self.inner.color()
    }

    fn set_color(&mut self, fg: Option<u8>, bg: Option<u8>) -> io::Result<()> {
        self.inner.set_color(fg, bg)
    }

    fn enter_alt(&mut self) -> io::Result<()> {
        self.inner.enter_alt()
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }

    fn is_interactive(&self) -> bool {
        self.inner.is_interactive()
    }

    fn leave_alt(&mut self) -> io::Result<()> {
        self.inner.leave_alt()
    }

    fn locate(&mut self, pos: CharsXY) -> io::Result<()> {
        self.inner.locate(pos)
    }

    fn move_within_line(&mut self, off: i16) -> io::Result<()> {
        self.inner.move_within_line(off)
    }

    fn print(&mut self, text: &str) -> io::Result<()> {
        self.inner.print(text)
    }

    async fn poll_key(&mut self) -> io::Result<Option<Key>> {
        self.inner.poll_key().await
    }

    async fn read_key(&mut self) -> io::Result<Key> {
        self.inner.read_key().await
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.inner.show_cursor()
    }

    fn size_chars(&self) -> io::Result<CharsXY> {
        self.inner.size_chars()
    }

    fn size_pixels(&self) -> io::Result<SizeInPixels> {
        self.inner.size_pixels()
    }

    fn write(&mut self, text: &str) -> io::Result<()> {
        self.inner.write(text)
    }

    fn draw_circle(&mut self, center: PixelsXY, radius: u16) -> io::Result<()> {
        self.inner.draw_circle(center, radius)
    }

    fn draw_circle_filled(&mut self, center: PixelsXY, radius: u16) -> io::Result<()> {
        self.inner.draw_circle_filled(center, radius)
    }

    fn draw_line(&mut self, x1y1: PixelsXY, x2y2: PixelsXY) -> io::Result<()> {
        self.inner.draw_line(x1y1, x2y2)
    }

    fn draw_pixel(&mut self, xy: PixelsXY) -> io::Result<()> {
        self.inner.draw_pixel(xy)
    }

    fn draw_rect(&mut self, x1y1: PixelsXY, x2y2: PixelsXY) -> io::Result<()> {
        self.inner.draw_rect(x1y1, x2y2)
    }

    fn draw_rect_filled(&mut self, x1y1: PixelsXY, x2y2: PixelsXY) -> io::Result<()> {
        self.inner.draw_rect_filled(x1y1, x2y2)
    }

    fn sync_now(&mut self) -> io::Result<()> {
        self.inner.sync_now()
    }

    fn set_sync(&mut self, enabled: bool) -> io::Result<bool> {
        self.inner.set_sync(enabled)
    }
}

/// Initializes a new console on a ST7735S LCD.
pub fn new_st7735s_console(signals_tx: Sender<Signal>) -> io::Result<ST7735SConsole> {
    let mut gpio = Gpio::new().map_err(gpio_error_to_io_error)?;

    let lcd = ST7735SLcd::new(&mut gpio)?;
    let input = ST7735SInput::new(signals_tx)?;
    let lcd = BufferedLcd::new(lcd);
    let inner = GraphicsConsole::new(input, lcd)?;
    Ok(ST7735SConsole { _gpio: gpio, inner })
}
