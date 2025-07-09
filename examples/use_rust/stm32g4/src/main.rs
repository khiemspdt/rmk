#![no_main]
#![no_std]

#[macro_use]
mod macros;
mod keymap;
mod vial;

use defmt::info;
use embassy_executor::Spawner;
use embassy_stm32::Peripheral;
use embassy_stm32::adc::{Adc, AdcChannel as _, SampleTime};
use embassy_stm32::gpio::{Input, Output};
use embassy_stm32::peripherals::USB;
use embassy_stm32::usb::{Driver, InterruptHandler};
use embassy_stm32::{Config, bind_interrupts};
use embassy_time::Timer;
use keymap::{COL, ROW};
use rmk::channel::EVENT_CHANNEL;
use rmk::config::{BehaviorConfig, ControllerConfig, RmkConfig, VialConfig};
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::futures::future::join3;
use rmk::input_device::Runnable;
use rmk::keyboard::Keyboard;
use rmk::light::LightController;
use rmk::matrix::Matrix;
use rmk::{initialize_keymap, run_devices, run_rmk};
use vial::{VIAL_KEYBOARD_DEF, VIAL_KEYBOARD_ID};
use {defmt_rtt as _, panic_halt as _};
bind_interrupts!(struct Irqs {
    USB_LP => InterruptHandler<USB>;
});

static mut DMA_BUF: [u16; 2] = [0; 2];

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let mut read_buffer = unsafe { &mut DMA_BUF[..] };

    info!("RMK start!");
    // RCC config
    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.pll = Some(Pll {
            source: PllSource::HSI,
            prediv: PllPreDiv::DIV4,
            mul: PllMul::MUL85,
            divp: None,
            divq: None,
            // Main system clock at 170 MHz
            divr: Some(PllRDiv::DIV2),
        });
        config.rcc.mux.adc12sel = mux::Adcsel::SYS;
        config.rcc.sys = Sysclk::PLL1_R;
    }

    // Initialize peripherals
    let p = embassy_stm32::init(config);

    // // Configure PA4 as an analog pin
    // let mut adc = Adc::new(p.ADC2);
    // let mut dma = p.DMA1_CH2;
    // let mut adc_s0 = p.PA4.degrade_adc();
    // let mut adc_s1 = p.PA5.degrade_adc();

    // loop {
    //     adc.read(
    //         &mut dma,
    //         [
    //             (&mut adc_s0, SampleTime::CYCLES247_5),
    //             (&mut adc_s1, SampleTime::CYCLES247_5),
    //         ]
    //         .into_iter(),
    //         &mut read_buffer,
    //     )
    //     .await;

    //     let vrefint = read_buffer[0];
    //     let measured = read_buffer[1];
    //     info!("vrefint: {}", vrefint);
    //     info!("measured: {}", measured);
    //     Timer::after_millis(500).await;
    // }

    // Pin config
    let (input_pins, output_pins) =
        config_matrix_pins_stm32!(peripherals: p, input: [PA4, PA5, PA6, PA7], output: [PB11,PB12, PB13, PB14]);

    // Usb driver
    let driver = Driver::new(p.USB, Irqs, p.PA12, p.PA11);

    // Keyboard config
    let rmk_config = RmkConfig {
        vial_config: VialConfig::new(VIAL_KEYBOARD_ID, VIAL_KEYBOARD_DEF),
        ..Default::default()
    };

    // Initialize the keymap
    let mut default_keymap = keymap::get_default_keymap();
    let behavior_config = BehaviorConfig::default();
    // let storage_config = StorageConfig::default();

    let keymap = initialize_keymap(&mut default_keymap, behavior_config).await;

    // --- MuxMatrix Example for STM32G4/Embassy ---
    #[cfg(feature = "muxmatrix")]
    {
        const NUM_OUTPUTS: usize = 4; // Number of ADC input pins
        const MUX_COUNT: usize = 2; // Number of MUX chips
        const CHANNEL_COUNT: usize = 16; // Number of channels per MUX
        use defmt::info;
        use embassy_stm32::adc::{Adc, AnyAdcChannel};
        use embassy_stm32::gpio::Output;
        use embassy_stm32::peripherals::{ADC2, DMA1_CH2};
        use embassy_time::Timer;
        use rmk::debounce::default_debouncer::DefaultDebouncer;
        use rmk::matrix::MuxMatrix;

        // S0-S3: PB11, PB12, PB13, PB14
        let mut s0 = Output::new(p.PB11, embassy_stm32::gpio::Level::Low, embassy_stm32::gpio::Speed::Low);
        let mut s1 = Output::new(p.PB12, embassy_stm32::gpio::Level::Low, embassy_stm32::gpio::Speed::Low);
        let mut s2 = Output::new(p.PB13, embassy_stm32::gpio::Level::Low, embassy_stm32::gpio::Speed::Low);
        let mut s3 = Output::new(p.PB14, embassy_stm32::gpio::Level::Low, embassy_stm32::gpio::Speed::Low);
        let mut s_pins = [s0, s1, s2, s3];

        // COM pins: PA4, PA5, PA6, PA7 (use AnyAdcChannel<ADC2> for type erasure)
        let mut adc = Adc::new(p.ADC2);
        let mut dma = p.DMA1_CH2;
        let mut adc_s0 = p.PA4.degrade_adc();
        let mut adc_s1 = p.PA5.degrade_adc();
        let mut adc_s2 = p.PA6.degrade_adc();
        let mut adc_s3 = p.PA7.degrade_adc();
        let mut input_pins = [adc_s0, adc_s1, adc_s2, adc_s3]; // [AnyAdcChannel<ADC2>; NUM_INPUTS]

        let debouncer = DefaultDebouncer::<MUX_COUNT, CHANNEL_COUNT>::new();
        let threshold: u16 = 2000;
        let mut matrix = MuxMatrix::<
            Output,
            Adc<ADC2>,
            DMA1_CH2,
            AnyAdcChannel<ADC2>,
            _,
            NUM_OUTPUTS,
            MUX_COUNT,
            CHANNEL_COUNT,
        >::new(s_pins, adc, dma, input_pins, debouncer, threshold);

        let mut keyboard = Keyboard::new(&keymap);
        let mut light_controller: LightController<Output> =
            LightController::new(ControllerConfig::default().light_config);

        join3(
            keyboard.run(),
            run_rmk(&keymap, driver, &mut light_controller, rmk_config),
            run_devices! (
                (matrix) => EVENT_CHANNEL,
            ),
        )
        .await;

        // Main async loop: scan the mux matrix and handle key events
        loop {
            if let Some(event) = matrix.scan_and_update().await {
                info!("Key event: {:?}", event);
                // You can also send the event to your event channel or process as needed
            }
            Timer::after_millis(1).await;
        }

        return;
    }
    // --- End MuxMatrix Example ---

    // Initialize the matrix + keyboard (default, non-muxmatrix)
    let debouncer = DefaultDebouncer::<ROW, COL>::new();
    let mut matrix = Matrix::<_, _, _, ROW, COL>::new(input_pins, output_pins, debouncer);
    let mut keyboard = Keyboard::new(&keymap);

    // Initialize the light controller
    let mut light_controller: LightController<Output> = LightController::new(ControllerConfig::default().light_config);

    // Start
    join3(
        keyboard.run(),
        run_rmk(&keymap, driver, &mut light_controller, rmk_config),
        run_devices! (
            (matrix) => EVENT_CHANNEL,
        ),
    )
    .await;
}
