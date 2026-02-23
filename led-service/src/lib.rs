#![no_std]

use core::pin::pin;

use embedded_hal::digital::OutputPin;
use ergot::endpoint;
use shared_icd::Network;

// LED service
endpoint!(LedEndpoint, bool, (), "led/set");

pub async fn led_service<O: OutputPin>(
    net_stack: &'static Network,
    name: &'static str,
    mut led: O,
) -> ! {
    let socket = net_stack
        .endpoints()
        .bounded_server::<LedEndpoint, 2>(Some(name));
    let socket = pin!(socket);
    let mut hdl = socket.attach();
    loop {
        let _ = hdl
            .serve(async |on| {
                if *on {
                    led.set_low().unwrap();
                } else {
                    led.set_high().unwrap();
                }
            })
            .await;
    }
}
