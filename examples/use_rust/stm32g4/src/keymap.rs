use rmk::action::KeyAction;
use rmk::{a, k, layer, mo};
pub(crate) const COL: usize = 4;
pub(crate) const ROW: usize = 4;
pub(crate) const NUM_LAYER: usize = 2;

#[rustfmt::skip]
pub const fn get_default_keymap() -> [[[KeyAction; COL]; ROW]; NUM_LAYER] {
    [
        layer!([
            [k!(AudioVolUp), k!(B), k!(AudioVolDown), k!(Kp0)],
            [k!(Kp4), k!(LShift), k!(Kp6), k!(Kp1)],
            [mo!(1), k!(Kp2), k!(Kp3), k!(Kp2)],
            [k!(Kp7), k!(Kp8), k!(Kp9), k!(Kp4)]
        ]),
        layer!([
            [k!(Kp7), k!(Kp8), k!(Kp9), k!(Kp4)],
            [k!(Kp4), k!(LCtrl), k!(Kp6), k!(Kp5)],
            [mo!(1), k!(Kp2), k!(Kp3), k!(Kp6)],
            [k!(Kp7), k!(Kp8), k!(Kp9), k!(Kp4)]
        ]),
    ]
}
