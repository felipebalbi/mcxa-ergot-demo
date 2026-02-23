#![no_std]

use core::pin::pin;

use embedded_hal::pwm::SetDutyCycle;
use ergot::endpoint;
use shared_icd::Network;

endpoint!(PwmEndpoint, (u8, u8), (), "pwm/brightness");

pub async fn pwm_service<D: SetDutyCycle>(
    net_stack: &'static Network,
    mut red: D,
    mut blue: D,
) -> ! {
    let socket = net_stack.endpoints().bounded_server::<PwmEndpoint, 2>(None);
    let socket = pin!(socket);
    let mut hdl = socket.attach();
    loop {
        let _ = hdl
            .serve(async |(r, b)| {
                red.set_duty_cycle_percent(*r).unwrap();
                blue.set_duty_cycle_percent(*b).unwrap();
            })
            .await;
    }
}
