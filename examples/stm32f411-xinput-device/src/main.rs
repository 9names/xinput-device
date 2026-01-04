#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::time::Hertz;
use embassy_stm32::usb::Driver;
use embassy_stm32::{Config, bind_interrupts, peripherals, usb};
use embassy_time::Timer;

use {defmt_rtt as _, panic_probe as _};

use static_cell::StaticCell;

static CONTROLLER_STATE: [xinput::State; 4] = [const { xinput::State::new() }; 4];

use xinput_device::{
    controller::XboxGamepad,
    xinput::{self, XInput},
};

type UsbDriver = embassy_stm32::usb::Driver<'static, peripherals::USB_OTG_FS>;
type UsbDevice = embassy_usb::UsbDevice<'static, UsbDriver>;

bind_interrupts!(struct Irqs {
    OTG_FS => usb::InterruptHandler<peripherals::USB_OTG_FS>;
});

#[embassy_executor::task]
async fn usb_task(mut usb: UsbDevice) -> ! {
    usb.run().await
}

#[embassy_executor::task]
async fn xinput_task(xinput_device: XInput<'static, UsbDriver>) -> ! {
    xinput_device.run().await
}

#[embassy_executor::task]
async fn controller_state_task() -> ! {
    let mut a_pressed = false;
    loop {
        let controller_state = XboxGamepad {
            btn_a: a_pressed,
            ..Default::default()
        };
        CONTROLLER_STATE[0].send_xinput(controller_state.into());
        a_pressed = !a_pressed;
        Timer::after_secs(1).await;
    }
}

// If you are trying this and your USB device doesn't connect, the most
// common issues are the RCC config and vbus_detection
//
// See https://embassy.dev/book/#_the_usb_examples_are_not_working_on_my_board_is_there_anything_else_i_need_to_configure
// for more information.
#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Hello World!");
    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.hse = Some(Hse {
            freq: Hertz(25_000_000),
            mode: HseMode::Oscillator,
        });
        config.rcc.pll_src = PllSource::HSE;
        config.rcc.pll = Some(Pll {
            prediv: PllPreDiv::DIV25,
            mul: PllMul::MUL192,
            divp: Some(PllPDiv::DIV2), // 25mhz / 25 * 192 / 2 = 96Mhz.
            divq: Some(PllQDiv::DIV4), // 25mhz / 25 * 192 / 4 = 48Mhz.
            divr: Some(PllRDiv::DIV2),
        });
        config.rcc.ahb_pre = AHBPrescaler::DIV1;
        config.rcc.apb1_pre = APBPrescaler::DIV2;
        config.rcc.apb2_pre = APBPrescaler::DIV1;
        config.rcc.sys = Sysclk::PLL1_P;

        config.rcc.mux.clk48sel = mux::Clk48sel::PLL1_Q;
    }
    let p = embassy_stm32::init(config);
    info!("Initialized");

    // Create the driver, from the HAL.
    static EP_OUT_BUFFER: StaticCell<[u8; 256]> = StaticCell::new();
    let mut config = embassy_stm32::usb::Config::default();

    // Do not enable vbus_detection. This is a safe default that works in all boards.
    // However, if your USB device is self-powered (can stay powered on if USB is unplugged), you need
    // to enable vbus_detection to comply with the USB spec. If you enable it, the board
    // has to support it or USB won't work at all. See docs on `vbus_detection` for details.
    config.vbus_detection = false;

    let driver = Driver::new_fs(
        p.USB_OTG_FS,
        Irqs,
        p.PA12,
        p.PA11,
        EP_OUT_BUFFER.init([0u8; 256]),
        config,
    );

    // Create embassy-usb Config (use Xbox 360 Wireless Receiver VID/PID)
    let mut config = embassy_usb::Config::new(0x045e, 0x0719);
    config.composite_with_iads = false;
    config.device_class = 0x00;
    config.device_sub_class = 0x00;
    config.device_protocol = 0x00;

    config.device_release = 0x0114;
    config.manufacturer = Some("Microsoft");
    config.product = Some("Xbox 360 Wireless Receiver");
    config.serial_number = Some("FFFFFFFF");
    config.max_power = 260;
    config.max_packet_size_0 = 64;

    // The first 4 bytes should match the USB serial number descriptor.
    // Not required for the receiver to be detected by the windows driver.
    static SERIAL_NUMBER_HANDLER: StaticCell<xinput::SerialNumberHandler> = StaticCell::new();
    let mut builder = {
        static CONFIG_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
        static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
        static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();

        embassy_usb::Builder::new(
            driver,
            config,
            CONFIG_DESCRIPTOR.init([0; 256]),
            BOS_DESCRIPTOR.init([0; 256]),
            &mut [], // no msos descriptors
            CONTROL_BUF.init([0; 64]),
        )
    };

    let x = xinput::SerialNumberHandler([0xFF, 0xFF, 0xFF, 0xFF, 0x0a, 0x89, 0xB7]);
    builder.handler(SERIAL_NUMBER_HANDLER.init(x));

    let controller_0 = XInput::new_wireless(&mut builder, &CONTROLLER_STATE[0], false);

    let usb = builder.build();
    unwrap!(spawner.spawn(usb_task(usb)));
    unwrap!(spawner.spawn(xinput_task(controller_0)));
    unwrap!(spawner.spawn(controller_state_task()));

    loop {
        info!("loop");
        Timer::after_secs(1).await;
    }
}
