use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};

use defmt::{debug, unwrap};
use embassy_futures::select::{select, Either};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_usb::control::{InResponse, Request, RequestType};
use embassy_usb::driver::{Driver, Endpoint, EndpointIn, EndpointOut};
use embassy_usb::Handler;

pub struct SerialNumberHandler(pub [u8; 7]);

impl Handler for SerialNumberHandler {
    fn control_in<'a>(&'a mut self, req: Request, buf: &'a mut [u8]) -> Option<InResponse<'a>> {
        if req.request_type == RequestType::Vendor
            && req.request == 1
            && req.value == 1
            && req.index == 0
            && req.length >= self.0.len() as u16
        {
            buf[..self.0.len()].copy_from_slice(&self.0);
            Some(InResponse::Accepted(&buf[..self.0.len()]))
        } else {
            None
        }
    }
}

#[derive(Default)]
pub struct State {
    available: AtomicBool,
    xinput: Signal<CriticalSectionRawMutex, [u8; 12]>,
    // right (weak) rumble in high byte
    // left (strong) rumble in low byte
    rumble: AtomicU16,
}

impl State {
    pub const fn new() -> Self {
        State {
            available: AtomicBool::new(false),
            xinput: Signal::new(),
            rumble: AtomicU16::new(0),
        }
    }

    pub fn set_available(&self, available: bool) {
        self.available.store(available, Ordering::Relaxed);
    }

    pub fn send_xinput(&self, data: [u8; 12]) {
        self.xinput.signal(data);
    }

    // Returns the (strong, weak) rumble data pair.
    pub fn rumble(&self) -> (u8, u8) {
        let [strong, weak] = self.rumble.load(Ordering::Relaxed).to_le_bytes();
        (strong, weak)
    }
}

pub struct XInput<'d, D: Driver<'d>> {
    ep_in: D::EndpointIn,
    ep_out: D::EndpointOut,
    state: &'d State,
}

impl<'d, D: Driver<'d>> XInput<'d, D> {
    pub fn new_wireless(builder: &mut embassy_usb::Builder<'d, D>, state: &'d State) -> Self {
        const CLASS_VENDOR: u8 = 0xFF;
        const SUBCLASS_XINPUT: u8 = 0x5D;
        const PROTOCOL_WIRELESS: u8 = 0x81;
        const PROTOCOL_WIRELESS_UNKNOWN: u8 = 0x82;
        let mut function = builder.function(CLASS_VENDOR, SUBCLASS_XINPUT, PROTOCOL_WIRELESS);
        let mut interface = function.interface();
        let mut alt = interface.alt_setting(CLASS_VENDOR, SUBCLASS_XINPUT, PROTOCOL_WIRELESS, None);

        let ep_in = alt.endpoint_interrupt_in(32, 1);
        let ep_in_idx = 0x80 | ep_in.info().addr.index() as u8;
        let ep_out = alt.endpoint_interrupt_out(32, 8);
        let ep_out_idx = ep_out.info().addr.index() as u8;

        // Unknown descriptor
        alt.descriptor(
            0x22,
            &[
                // Unknown
                0x00,
                0x01,
                // Endpoint information
                0x13,             // type = 1, length = 3
                0x80 | ep_in_idx, // IN endpoint
                0x1D,             // IN data size
                0x00,             // ?
                0x17,             // IN data used
                // Unknown
                0x01,
                0x02,
                0x08,
                // Endpoint information
                0x13,       // type = 1, length 3
                ep_out_idx, // OUT endpoint
                0x0C,       // OUT max data size
                0x00,       // ?
                0x0C,       // OUT data used
                // Unknown
                0x01,
                0x02,
                0x08,
            ],
        );

        // Unused unknown interface
        {
            drop(function);
            let mut function =
                builder.function(CLASS_VENDOR, SUBCLASS_XINPUT, PROTOCOL_WIRELESS_UNKNOWN);
            let mut interface = function.interface();
            let mut alt = interface.alt_setting(
                CLASS_VENDOR,
                SUBCLASS_XINPUT,
                PROTOCOL_WIRELESS_UNKNOWN,
                None,
            );

            let ep_in = alt.endpoint_interrupt_in(32, 2);
            let ep_in_idx = 0x80 | ep_in.info().addr.index() as u8;
            let ep_out = alt.endpoint_interrupt_out(32, 4);
            let ep_out_idx = ep_out.info().addr.index() as u8;

            alt.descriptor(
                0x22,
                &[
                    0x00,
                    0x01,
                    0x01,
                    0x80 | ep_in_idx,
                    0x00,
                    0x40,
                    0x01,
                    ep_out_idx,
                    0x20,
                    0x00,
                ],
            );
        }

        Self {
            ep_in,
            ep_out,
            state,
        }
    }

    async fn send_status(&mut self) {
        let data: [u8; 2] = if self.state.available.load(Ordering::Relaxed) {
            [0x08, 0x80]
        } else {
            [0x08, 0x00]
        };
        unwrap!(self.ep_in.write(&data).await);
    }

    pub async fn run(mut self) -> ! {
        let mut out_data = [0_u8; 32];
        loop {
            match select(self.state.xinput.wait(), self.ep_out.read(&mut out_data)).await {
                Either::First(xinput_data) => {
                    let mut data = [0_u8; 29];
                    data[0] = 0x00; // Outer message type?
                    data[1] = 0x01; // Message contains xinput data
                    data[4] = 0x00; // Inner message type
                    data[5] = 0x13; // Inner message length
                    data[6..18].copy_from_slice(&xinput_data);
                    unwrap!(self.ep_in.write(&data).await);
                }
                Either::Second(n) => {
                    let out_data = &out_data[..unwrap!(n)];
                    debug!("OUT DATA: {=[u8]:X}", out_data);

                    if out_data.len() >= 4 && &out_data[..4] == &[0x08, 0x00, 0x0f, 0xc0] {
                        // Status request
                        self.send_status().await;
                    } else if out_data.len() >= 7
                        && &out_data[..5] == &[0x00, 0x01, 0x0F, 0xC0, 0x00]
                    {
                        // Rumble data
                        let rumble16 = u16::from_le_bytes([out_data[5], out_data[6]]);
                        self.state.rumble.store(rumble16, Ordering::Relaxed);
                    }
                }
            }
        }
    }
}