use core::future::Future;
use core::sync::atomic::Ordering;

use embassy_time::{Instant, Timer};
use embedded_hal::digital::{InputPin, OutputPin};
#[cfg(feature = "async_matrix")]
use {embassy_futures::select::select_slice, embedded_hal_async::digital::Wait, heapless::Vec};

use crate::debounce::{DebounceState, DebouncerTrait};
use crate::event::{Event, KeyEvent};
use crate::input_device::InputDevice;
use crate::state::ConnectionState;
use crate::CONNECTION_STATE;

/// MatrixTrait is the trait for keyboard matrix.
///
/// The keyboard matrix is a 2D matrix of keys, the matrix does the scanning and saves the result to each key's `KeyState`.
/// The `KeyState` at position (row, col) can be read by `get_key_state` and updated by `update_key_state`.
pub trait MatrixTrait: InputDevice {
    // Matrix size
    const ROW: usize;
    const COL: usize;

    // Wait for USB or BLE really connected
    fn wait_for_connected(&self) -> impl Future<Output = ()> {
        async {
            while CONNECTION_STATE.load(Ordering::Acquire) == ConnectionState::Disconnected.into() {
                embassy_time::Timer::after_millis(100).await;
            }
            info!("Connected, start scanning matrix");
        }
    }

    #[cfg(feature = "async_matrix")]
    fn wait_for_key(&mut self) -> impl Future<Output = ()>;
}

/// KeyState represents the state of a key.
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct KeyState {
    // True if the key is pressed
    pub pressed: bool,
    // True if the key's state is just changed
    // pub changed: bool,
}

impl Default for KeyState {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyState {
    pub fn new() -> Self {
        KeyState { pressed: false }
    }

    pub fn toggle_pressed(&mut self) {
        self.pressed = !self.pressed;
    }

    pub fn is_releasing(&self) -> bool {
        !self.pressed
    }

    pub fn is_pressing(&self) -> bool {
        self.pressed
    }
}

/// Matrix is the physical pcb layout of the keyboard matrix.
pub struct Matrix<
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    Out: OutputPin,
    D: DebouncerTrait,
    const INPUT_PIN_NUM: usize,
    const OUTPUT_PIN_NUM: usize,
> {
    /// Input pins of the pcb matrix
    input_pins: [In; INPUT_PIN_NUM],
    /// Output pins of the pcb matrix
    output_pins: [Out; OUTPUT_PIN_NUM],
    /// Debouncer
    debouncer: D,
    /// Key state matrix
    key_states: [[KeyState; INPUT_PIN_NUM]; OUTPUT_PIN_NUM],
    /// Start scanning
    scan_start: Option<Instant>,
    /// Current scan pos: (out_idx, in_idx)
    scan_pos: (usize, usize),
}

impl<
        #[cfg(not(feature = "async_matrix"))] In: InputPin,
        #[cfg(feature = "async_matrix")] In: Wait + InputPin,
        Out: OutputPin,
        D: DebouncerTrait,
        const INPUT_PIN_NUM: usize,
        const OUTPUT_PIN_NUM: usize,
    > Matrix<In, Out, D, INPUT_PIN_NUM, OUTPUT_PIN_NUM>
{
    /// Create a matrix from input and output pins.
    pub fn new(input_pins: [In; INPUT_PIN_NUM], output_pins: [Out; OUTPUT_PIN_NUM], debouncer: D) -> Self {
        Matrix {
            input_pins,
            output_pins,
            debouncer,
            key_states: [[KeyState::new(); INPUT_PIN_NUM]; OUTPUT_PIN_NUM],
            scan_start: None,
            scan_pos: (0, 0),
        }
    }
}

impl<
        #[cfg(not(feature = "async_matrix"))] In: InputPin,
        #[cfg(feature = "async_matrix")] In: Wait + InputPin,
        Out: OutputPin,
        D: DebouncerTrait,
        const INPUT_PIN_NUM: usize,
        const OUTPUT_PIN_NUM: usize,
    > InputDevice for Matrix<In, Out, D, INPUT_PIN_NUM, OUTPUT_PIN_NUM>
{
    async fn read_event(&mut self) -> crate::event::Event {
        loop {
            let (out_idx_start, in_idx_start) = self.scan_pos;
            #[cfg(feature = "async_matrix")]
            self.wait_for_key().await;

            // Scan matrix and send report
            for out_idx in out_idx_start..self.output_pins.len() {
                // Pull up output pin, wait 1us ensuring the change comes into effect
                if let Some(out_pin) = self.output_pins.get_mut(out_idx) {
                    out_pin.set_high().ok();
                }
                Timer::after_micros(1).await;
                for in_idx in in_idx_start..self.input_pins.len() {
                    let in_pin = self.input_pins.get_mut(in_idx).unwrap();
                    // Check input pins and debounce
                    let debounce_state = self.debouncer.detect_change_with_debounce(
                        in_idx,
                        out_idx,
                        in_pin.is_high().ok().unwrap_or_default(),
                        &self.key_states[out_idx][in_idx],
                    );

                    if let DebounceState::Debounced = debounce_state {
                        self.key_states[out_idx][in_idx].toggle_pressed();
                        #[cfg(feature = "col2row")]
                        let (row, col, key_state) = (in_idx, out_idx, self.key_states[out_idx][in_idx]);
                        #[cfg(not(feature = "col2row"))]
                        let (row, col, key_state) = (out_idx, in_idx, self.key_states[out_idx][in_idx]);

                        self.scan_pos = (out_idx, in_idx);
                        return Event::Key(KeyEvent {
                            row: row as u8,
                            col: col as u8,
                            pressed: key_state.pressed,
                        });
                    }

                    // If there's key still pressed, always refresh the self.scan_start
                    #[cfg(feature = "async_matrix")]
                    if self.key_states[out_idx][in_idx].pressed {
                        self.scan_start = Some(Instant::now());
                    }
                }

                // Pull it back to low
                if let Some(out_pin) = self.output_pins.get_mut(out_idx) {
                    out_pin.set_low().ok();
                }
            }
            self.scan_pos = (0, 0);
        }
    }
}

impl<
        #[cfg(not(feature = "async_matrix"))] In: InputPin,
        #[cfg(feature = "async_matrix")] In: Wait + InputPin,
        Out: OutputPin,
        D: DebouncerTrait,
        const INPUT_PIN_NUM: usize,
        const OUTPUT_PIN_NUM: usize,
    > MatrixTrait for Matrix<In, Out, D, INPUT_PIN_NUM, OUTPUT_PIN_NUM>
{
    #[cfg(feature = "col2row")]
    const ROW: usize = INPUT_PIN_NUM;
    #[cfg(feature = "col2row")]
    const COL: usize = OUTPUT_PIN_NUM;
    #[cfg(not(feature = "col2row"))]
    const ROW: usize = OUTPUT_PIN_NUM;
    #[cfg(not(feature = "col2row"))]
    const COL: usize = INPUT_PIN_NUM;

    #[cfg(feature = "async_matrix")]
    async fn wait_for_key(&mut self) {
        use core::pin::pin;

        if let Some(start_time) = self.scan_start {
            // If no key press over 1ms, stop scanning and wait for interupt
            if start_time.elapsed().as_millis() <= 1 {
                return;
            } else {
                self.scan_start = None;
            }
        }
        // First, set all output pin to high
        for out in self.output_pins.iter_mut() {
            out.set_high().ok();
        }
        Timer::after_micros(1).await;
        let mut futs: Vec<_, INPUT_PIN_NUM> = self
            .input_pins
            .iter_mut()
            .map(|input_pin| input_pin.wait_for_high())
            .collect();
        let _ = select_slice(pin!(futs.as_mut_slice())).await;

        // Set all output pins back to low
        for out in self.output_pins.iter_mut() {
            out.set_low().ok();
        }

        self.scan_start = Some(Instant::now());
    }
}

#[cfg(feature = "muxmatrix")]
/// Platform-agnostic MuxMatrix for CD74HC4067 multiplexers.
pub struct MuxMatrix<
    S: OutputPin,
    ADC,
    DMA,
    P,
    D: DebouncerTrait,
    const NUM_OUTPUTS: usize,
    const MUX_COUNT: usize,
    const CHANNEL_COUNT: usize,
> {
    /// S0-S3 output pins (shared by all MUX chips)
    s_pins: [S; NUM_OUTPUTS],
    /// ADC peripheral
    adc: ADC,
    /// DMA channel
    dma: DMA,
    /// MUX COM analog input pins (each is a Channel)
    input_pins: [P; MUX_COUNT],
    /// Debouncer
    debouncer: D,
    /// Key state matrix
    key_states: [[KeyState; CHANNEL_COUNT]; MUX_COUNT],
    /// ADC read buffer
    read_buffer: [u16; MUX_COUNT],
    /// Hardcoded threshold
    threshold: u16,
}

#[cfg(feature = "muxmatrix")]
impl<
        S: OutputPin,
        ADC,
        DMA,
        P,
        D: DebouncerTrait,
        const NUM_OUTPUTS: usize,
        const MUX_COUNT: usize,
        const CHANNEL_COUNT: usize,
    > MuxMatrix<S, ADC, DMA, P, D, NUM_OUTPUTS, MUX_COUNT, CHANNEL_COUNT>
{
    pub fn new(
        s_pins: [S; NUM_OUTPUTS],
        adc: ADC,
        dma: DMA,
        input_pins: [P; MUX_COUNT],
        debouncer: D,
        threshold: u16,
    ) -> Self {
        Self {
            s_pins,
            adc,
            dma,
            input_pins,
            debouncer,
            key_states: [[KeyState::new(); CHANNEL_COUNT]; MUX_COUNT],
            read_buffer: [0; MUX_COUNT],
            threshold,
        }
    }

    /// Set S0-S3 pins to select a channel (0..CHANNEL_COUNT-1)
    fn set_channel(&mut self, channel: usize) {
        for (i, pin) in self.s_pins.iter_mut().enumerate() {
            if ((channel >> i) & 1) == 1 {
                let _ = pin.set_high();
            } else {
                let _ = pin.set_low();
            }
        }
    }
}

#[cfg(feature = "muxmatrix")]
impl<'d, D: DebouncerTrait, const NUM_OUTPUTS: usize, const MUX_COUNT: usize, const CHANNEL_COUNT: usize>
    MuxMatrix<
        embassy_stm32::gpio::Output<'d>,
        embassy_stm32::adc::Adc<'d, embassy_stm32::peripherals::ADC2>,
        embassy_stm32::peripherals::DMA1_CH2,
        embassy_stm32::adc::AnyAdcChannel<embassy_stm32::peripherals::ADC2>,
        D,
        NUM_OUTPUTS,
        MUX_COUNT,
        CHANNEL_COUNT,
    >
{
    /// Platform-specific async scan and update for STM32G4/embassy-stm32
    pub async fn scan_and_update(&mut self) -> Option<crate::event::Event> {
        use embassy_stm32::adc::SampleTime;
        use heapless::Vec;
        for channel in 0..CHANNEL_COUNT {
            self.set_channel(channel);
            embassy_time::Timer::after_micros(1).await;
            let mut pin_refs: Vec<
                (
                    &mut embassy_stm32::adc::AnyAdcChannel<embassy_stm32::peripherals::ADC2>,
                    SampleTime,
                ),
                NUM_OUTPUTS,
            > = Vec::new();
            for pin in self.input_pins.iter_mut() {
                pin_refs.push((pin, SampleTime::CYCLES247_5)).ok();
            }
            self.adc
                .read(
                    &mut self.dma,
                    pin_refs.iter_mut().map(|(p, s)| (&mut **p, *s)),
                    &mut self.read_buffer,
                )
                .await;
            // For each input pin (MUX chip)
            for (mux_idx, &adc_value) in self.read_buffer.iter().enumerate() {
                let pressed = adc_value > self.threshold;
                let debounce_state = self.debouncer.detect_change_with_debounce(
                    mux_idx,
                    channel,
                    pressed,
                    &self.key_states[mux_idx][channel],
                );
                if let crate::debounce::DebounceState::Debounced = debounce_state {
                    self.key_states[mux_idx][channel].toggle_pressed();
                    return Some(crate::event::Event::Key(crate::event::KeyEvent {
                        row: mux_idx as u8,
                        col: channel as u8,
                        pressed: self.key_states[mux_idx][channel].pressed,
                    }));
                }
            }
        }
        None
    }
}

#[cfg(feature = "muxmatrix")]
#[allow(unused_mut)]
impl<'d, D: DebouncerTrait, const NUM_OUTPUTS: usize, const MUX_COUNT: usize, const CHANNEL_COUNT: usize> InputDevice
    for MuxMatrix<
        embassy_stm32::gpio::Output<'d>,
        embassy_stm32::adc::Adc<'d, embassy_stm32::peripherals::ADC2>,
        embassy_stm32::peripherals::DMA1_CH2,
        embassy_stm32::adc::AnyAdcChannel<embassy_stm32::peripherals::ADC2>,
        D,
        NUM_OUTPUTS,
        MUX_COUNT,
        CHANNEL_COUNT,
    >
{
    async fn read_event(&mut self) -> crate::event::Event {
        // Platform-specific: poll scan_and_update until an event is returned
        loop {
            if let Some(event) = self.scan_and_update().await {
                return event;
            }
            embassy_time::Timer::after_millis(1).await;
        }
    }
}

pub struct TestMatrix<const ROW: usize, const COL: usize> {
    last: bool,
}
impl<const ROW: usize, const COL: usize> Default for TestMatrix<ROW, COL> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const ROW: usize, const COL: usize> TestMatrix<ROW, COL> {
    pub fn new() -> Self {
        Self { last: false }
    }
}
impl<const ROW: usize, const COL: usize> MatrixTrait for TestMatrix<ROW, COL> {
    const ROW: usize = ROW;
    const COL: usize = COL;

    #[cfg(feature = "async_matrix")]
    fn wait_for_key(&mut self) -> impl Future<Output = ()> {
        async {}
    }
}

impl<const ROW: usize, const COL: usize> InputDevice for TestMatrix<ROW, COL> {
    async fn read_event(&mut self) -> Event {
        if self.last {
            embassy_time::Timer::after_millis(100).await;
        } else {
            embassy_time::Timer::after_secs(5).await;
        }
        self.last = !self.last;
        // info!("Read event: {:?}", self.last);
        Event::Key(KeyEvent {
            row: 0,
            col: 0,
            pressed: self.last,
        })
    }
}
