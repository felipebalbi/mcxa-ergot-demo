#![no_std]

use core::pin::pin;

use embedded_hal_async::delay::DelayNs;
use embedded_hal_async::i2c::I2c;
use ergot::{Address, topic};
use pwm_service::PwmEndpoint;
use shared_icd::Network;
use tmp108::Tmp108;

topic!(TemperatureTopic, f32, "temperature/latest");

pub async fn temperature_service(net_stack: &'static Network) -> ! {
    let recv = net_stack
        .topics()
        .bounded_receiver::<TemperatureTopic, 3>(None);
    let recv = pin!(recv);
    let mut recv = recv.subscribe();

    loop {
        let msg = recv.recv().await;
        defmt::info!("Temperature {=f32}C", msg.t);
    }
}

pub async fn tmp108_service<I2C: I2c, DELAY: DelayNs>(
    net_stack: &'static Network,
    mut tmp: Tmp108<I2C>,
    mut delay: DELAY,
) -> ! {
    let client = net_stack
        .endpoints()
        .client::<PwmEndpoint>(Address::unknown(), None);
    loop {
        let temperature = tmp.temperature().await.unwrap();
        let _ = net_stack
            .topics()
            .broadcast::<TemperatureTopic>(&temperature, None);

        // Convert to LED brightness. Maybe this should be done by the
        // PWM service itself.
        let mut temperature = temperature.clamp(18.0, 35.0);
        temperature -= 18.0;
        temperature /= 17.0;
        temperature *= 100.0;

        client
            .request(&(temperature as u8, (100.0 - temperature) as u8))
            .await
            .unwrap();
        delay.delay_ms(250).await;
    }
}
