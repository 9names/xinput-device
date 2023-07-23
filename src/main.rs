#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use core::cell::Cell;

use defmt::*;
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_rp::gpio::{Level, Output, Pull};
use embassy_rp::pio::{self, Direction, Pio, ShiftConfig, ShiftDirection};
use embassy_rp::relocate::RelocatedProgram;
use embassy_rp::{bind_interrupts, peripherals, usb, Peripheral};
use embassy_time::{with_timeout, Duration, Ticker};
use embassy_usb::class::hid;
use embassy_usb::driver::Driver;
use fixed::traits::ToFixed;
use fixed_macro::types::U56F8;
use {defmt_rtt as _, panic_probe as _};

type Controller<'d> = hid::HidWriter<'d, embassy_rp::usb::Driver<'d, peripherals::USB>, 6>;
type ControllerData = Cell<[u8; 6]>;

const CONTROLLER_DATA_INIT: ControllerData =
    ControllerData::new([0x00, 0x00, 0x80, 0x80, 0x80, 0x80]);

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<peripherals::USB>;
    PIO0_IRQ_0 => pio::InterruptHandler<peripherals::PIO0>;
});

// #[derive(Debug, Clone, Copy)]
// #[repr(packed)]
// struct ControllerData {
//     buttons: u16,
//     rx: u8,
//     ry: u8,
//     x: u8,
//     y: u8,
// }

fn build_hid_controller<'d, D: Driver<'d>>(
    builder: &mut embassy_usb::Builder<'d, D>,
    state: &'d mut hid::State<'d>,
) -> hid::HidWriter<'d, D, 6> {
    let config = hid::Config {
        #[rustfmt::skip]
        report_descriptor: &[
            0x05, 0x01,         // USAGE_PAGE (Generic Desktop)
            0x09, 0x05,         // USAGE (Game Pad)
            0xa1, 0x01,         // COLLECTION (Application)
            0x05, 0x09,         //   USAGE_PAGE (Button)
            0x09, 0x01,         //   USAGE (Button 1)
            0x09, 0x02,         //   USAGE (Button 2)
            0x09, 0x03,         //   USAGE (Button 3)
            0x09, 0x04,         //   USAGE (Button 4)
            0x09, 0x05,         //   USAGE (Button 5)
            0x09, 0x06,         //   USAGE (Button 6)
            0x09, 0x07,         //   USAGE (Button 7)
            0x09, 0x08,         //   USAGE (Button 8)
            0x09, 0x09,         //   USAGE (Button 9)
            0x09, 0x0a,         //   USAGE (Button 10)
            0x09, 0x0b,         //   USAGE (Button 11)
            0x09, 0x0c,         //   USAGE (Button 12)
            0x09, 0x0d,         //   USAGE (Button 13)
            0x09, 0x0e,         //   USAGE (Button 14)
            0x09, 0x0f,         //   USAGE (Button 15)
            0x09, 0x10,         //   USAGE (Button 16)
            0x15, 0x00,         //   LOGICAL_MINIMUM (0)
            0x25, 0x01,         //   LOGICAL_MAXIMUM (1)
            0x75, 0x01,         //   REPORT_SIZE (1)
            0x95, 0x10,         //   REPORT_COUNT (16)
            0x81, 0x02,         //   INPUT (Data,Var,Abs)
            0x05, 0x01,         //   USAGE_PAGE (Generic Desktop)
            0x09, 0x33,         //   USAGE (Rx)
            0x09, 0x34,         //   USAGE (Ry)
            0x09, 0x30,         //   USAGE (X)
            0x09, 0x31,         //   USAGE (Y)
            0x15, 0x00,         //   LOGICAL_MINIMUM (0)
            0x26, 0xff, 0x00,   //   LOGICAL_MAXIMUM (255)
            0x75, 0x08,         //   REPORT_SIZE (8)
            0x95, 0x04,         //   REPORT_COUNT (4)
            0x81, 0x02,         //   INPUT (Data,Var,Abs)
            0xc0,               // END_COLLECTION
        ],
        request_handler: None,
        poll_ms: 2,
        max_packet_size: 8,
    };
    hid::HidWriter::new(builder, state, config)
}

async fn run_controller(mut c: Controller<'_>, data: &ControllerData) {
    loop {
        let mut data = data.get();
        // PSX has buttons as active low
        data[0] = !data[0];
        data[1] = !data[1];
        //debug!("data={=[u8]:X}", data);
        unwrap!(c.write(data.as_ref()).await);
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let driver = usb::Driver::new(p.USB, Irqs);

    // Create embassy-usb Config
    let mut config = embassy_usb::Config::new(0x0000, 0x0000);
    config.device_release = 0x0000;
    config.manufacturer = Some("Timo Kröger");
    config.product = Some("psx-usb");
    config.max_power = 500;
    config.max_packet_size_0 = 64;

    // Required for Windows support.
    config.composite_with_iads = true;
    config.device_class = 0xEF;
    config.device_sub_class = 0x02;
    config.device_protocol = 0x01;

    // Create embassy-usb DeviceBuilder using the driver and config.
    // It needs some buffers for building the descriptors.
    let mut device_descriptor = [0; 256];
    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut control_buf = [0; 64];

    let mut s0 = hid::State::new();
    let mut s1 = hid::State::new();
    let mut s2 = hid::State::new();
    let mut s3 = hid::State::new();

    let mut builder = embassy_usb::Builder::new(
        driver,
        config,
        &mut device_descriptor,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut control_buf,
    );

    let c0 = build_hid_controller(&mut builder, &mut s0);
    let c1 = build_hid_controller(&mut builder, &mut s1);
    let c2 = build_hid_controller(&mut builder, &mut s2);
    let c3 = build_hid_controller(&mut builder, &mut s3);

    let mut usb = builder.build();

    let data = &[CONTROLLER_DATA_INIT; 4];

    // USB
    let fut = usb.run();

    // Controllers
    let fut = join(fut, async move { run_controller(c0, &data[0]).await });
    let fut = join(fut, async move { run_controller(c1, &data[1]).await });
    let fut = join(fut, async move { run_controller(c2, &data[2]).await });
    let fut = join(fut, async move { run_controller(c3, &data[3]).await });

    // PSX protocol: SPI with LSB first and special ack signal
    let fut = join(fut, async move {
        let Pio {
            mut common,
            irq_flags,
            mut sm0,
            mut sm1,
            ..
        } = Pio::new(p.PIO0, Irqs);

        let sck_clk = common.make_pio_pin(p.PIN_2);
        let txd_mosi = common.make_pio_pin(p.PIN_3);
        let mut rxd_miso = common.make_pio_pin(p.PIN_4);
        rxd_miso.set_pull(Pull::Up);
        let mut dtr_cs = Output::new(p.PIN_5, Level::High);
        let mut dsr_ack = common.make_pio_pin(p.PIN_6);
        dsr_ack.set_pull(Pull::Up);

        let psx_spi = pio_proc::pio_file!("src/psx.pio", select_program("spi"));
        let psx_spi = RelocatedProgram::new(&psx_spi.program);
        let psx_spi_len = psx_spi.code().count();
        let mut config = pio::Config::default();
        config.use_program(&common.load_program(&psx_spi), &[&sck_clk]);
        config.set_in_pins(&[&rxd_miso]);
        config.set_out_pins(&[&txd_mosi]);
        // 1MHz PIO clock gives SPI frequency of 250kHz
        config.clock_divider = (U56F8!(125_000_000) / U56F8!(1_000_000)).to_fixed();
        // LSB first
        config.shift_in = ShiftConfig {
            threshold: 32,
            direction: ShiftDirection::Right,
            auto_fill: true,
        };
        config.shift_out = ShiftConfig {
            threshold: 8,
            direction: ShiftDirection::Right,
            auto_fill: false,
        };
        sm0.set_config(&config);
        sm0.set_pin_dirs(Direction::Out, &[&sck_clk, &txd_mosi]);
        // Set register X to 0xFFFFFFFF with `mov x, !null`
        unsafe { sm0.exec_instr(0xa02b) };

        let psx_ack = pio_proc::pio_file!("src/psx.pio", select_program("ack"));
        let psx_ack = RelocatedProgram::new_with_origin(&psx_ack.program, psx_spi_len as u8);
        let mut config = pio::Config::default();
        config.use_program(&common.load_program(&psx_ack), &[]);
        config.set_in_pins(&[&dsr_ack]);
        sm1.set_config(&config);
        sm1.set_enable(true);

        let mut led = Output::new(p.PIN_25, Level::Low);

        let mut rx_dma = p.DMA_CH0.into_ref();
        let mut tx_dma = p.DMA_CH1.into_ref();

        let cmd_part1: [u8; 3] = [0x01, 0x42, 0x01];
        let cmd_part2: [u8; 32] = [
            0x42, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 1
            0x42, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 2
            0x42, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 3
            0x42, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 4
        ];

        let mut ticker = Ticker::every(Duration::from_millis(2));
        loop {
            ticker.next().await;

            dtr_cs.set_low();

            sm0.clear_fifos();
            irq_flags.set(0); // ACK flag
            sm0.tx().push(cmd_part1[0].into());
            sm0.tx().push(cmd_part1[1].into());
            sm0.tx().push(cmd_part1[2].into());
            sm0.set_enable(true);

            let mut rsp_part1 = [0x55_u8; 3];
            let _ = with_timeout(
                Duration::from_millis(1),
                sm0.rx().dma_pull(rx_dma.reborrow(), &mut rsp_part1),
            )
            .await;

            debug!("rsp_part1 = {=[u8]:X}", rsp_part1);
            if rsp_part1[1] == 0x80 && rsp_part1[2] == 0x5A {
                let mut rsp_part2 = [0_u8; 32];

                let (rx, tx) = sm0.rx_tx();
                let rx_fut = rx.dma_pull(rx_dma.reborrow(), &mut rsp_part2);
                let tx_fut = tx.dma_push(tx_dma.reborrow(), &cmd_part2);
                let _ = with_timeout(Duration::from_micros(1800), join(tx_fut, rx_fut)).await;

                data[0].set(rsp_part2[2..8].try_into().unwrap());
                data[1].set(rsp_part2[10..16].try_into().unwrap());
                data[2].set(rsp_part2[18..24].try_into().unwrap());
                data[3].set(rsp_part2[26..32].try_into().unwrap());

                led.toggle();
            }

            dtr_cs.set_high();
            sm0.set_enable(false);
        }
    });

    fut.await;
}

#[defmt::panic_handler]
fn defmt_panic() -> ! {
    cortex_m::asm::udf();
}
