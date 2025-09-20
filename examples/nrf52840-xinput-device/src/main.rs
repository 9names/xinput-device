#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_nrf::usb::Driver;
use embassy_nrf::usb::vbus_detect::HardwareVbusDetect;
use embassy_nrf::{bind_interrupts, pac, peripherals, usb};
use embassy_time::Timer;
use panic_probe as _;
use static_cell::StaticCell;

use xinput_device::{
    controller::XboxGamepad,
    xinput::{self, XInput},
};

static CONTROLLER_STATE: [xinput::State; 1] = [const { xinput::State::new() }; 1];

type UsbDriver = Driver<'static, peripherals::USBD, HardwareVbusDetect>;
type UsbDevice = embassy_usb::UsbDevice<'static, UsbDriver>;

bind_interrupts!(struct Irqs {
    USBD => usb::InterruptHandler<peripherals::USBD>;
    CLOCK_POWER => usb::vbus_detect::InterruptHandler;
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

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_nrf::init(Default::default());

    info!("Enabling ext hfosc...");
    pac::CLOCK.tasks_hfclkstart().write_value(1);
    while pac::CLOCK.events_hfclkstarted().read() != 1 {}

    // Create the driver, from the HAL.
    let driver = Driver::new(p.USBD, Irqs, HardwareVbusDetect::new(Irqs));

    // Create embassy-usb Config (use Xbox 360 Wireless Receiver VID/PID)
    let mut config = embassy_usb::Config::new(0x045E, 0x0719);

    config.composite_with_iads = false;
    config.device_class = 0xFF;
    config.device_sub_class = 0xFF;
    config.device_protocol = 0xFF;

    config.device_release = 0x0100;
    config.manufacturer = Some("Microsoft");
    config.product = Some("Xbox 360 Wireless Receiver");
    config.serial_number = Some("FFFFFFFF");
    config.max_power = 260;
    config.max_packet_size_0 = 64;

    // The first 4 bytes should match the USB serial number descriptor.
    // Not required for the receiver to be detected by the windows driver.
    static SERIAL_NUMBER_HANDLER: StaticCell<xinput::SerialNumberHandler> = StaticCell::new();
    let mut builder = {
        static CONFIG_DESCRIPTOR: StaticCell<[u8; 324]> = StaticCell::new();
        static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
        static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();
        embassy_usb::Builder::new(
            driver,
            config,
            CONFIG_DESCRIPTOR.init([0; 324]),
            BOS_DESCRIPTOR.init([0; 256]),
            &mut [], // no msos descriptors
            CONTROL_BUF.init([0; 64]),
        )
    };

    let x = xinput::SerialNumberHandler([0xFF, 0xFF, 0xFF, 0xFF, 0x0a, 0x89, 0xB7]);
    builder.handler(SERIAL_NUMBER_HANDLER.init(x));

    let controller_0 = XInput::new_wireless(&mut builder, &CONTROLLER_STATE[0], false);

    let usb = builder.build();
    let _usb_task_token = spawner.spawn(usb_task(usb));
    let _xinput_task_token = spawner.spawn(xinput_task(controller_0));
    let _controller_task_token = spawner.spawn(controller_state_task());

    loop {
        info!("loop");
        Timer::after_secs(1).await;
    }
}
