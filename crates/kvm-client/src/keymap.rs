//! Windows virtual-key code -> macOS virtual key code (kVK_*) mapping.
//!
//! The server is always Windows, so the wire format carries Windows VK codes
//! and this single table does all the translation.

use core_graphics::event::CGEventFlags;

pub struct MappedKey {
    /// macOS virtual key code (CGKeyCode).
    pub code: u16,
    /// Set when this key is a modifier; the injector maintains the event
    /// flags that must accompany every subsequent event.
    pub modifier: Option<CGEventFlags>,
}

fn key(code: u16) -> Option<MappedKey> {
    Some(MappedKey { code, modifier: None })
}

fn modifier(code: u16, flag: CGEventFlags) -> Option<MappedKey> {
    Some(MappedKey { code, modifier: Some(flag) })
}

/// `ctrl_as_cmd` maps the Windows Ctrl key onto macOS Command so that
/// Ctrl+C/V/X/Z/T/W behave like the usual macOS shortcuts.
pub fn map_vk(vk: u16, ctrl_as_cmd: bool) -> Option<MappedKey> {
    match vk {
        // --- Letters -------------------------------------------------------
        0x41 => key(0),   // A
        0x42 => key(11),  // B
        0x43 => key(8),   // C
        0x44 => key(2),   // D
        0x45 => key(14),  // E
        0x46 => key(3),   // F
        0x47 => key(5),   // G
        0x48 => key(4),   // H
        0x49 => key(34),  // I
        0x4A => key(38),  // J
        0x4B => key(40),  // K
        0x4C => key(37),  // L
        0x4D => key(46),  // M
        0x4E => key(45),  // N
        0x4F => key(31),  // O
        0x50 => key(35),  // P
        0x51 => key(12),  // Q
        0x52 => key(15),  // R
        0x53 => key(1),   // S
        0x54 => key(17),  // T
        0x55 => key(32),  // U
        0x56 => key(9),   // V
        0x57 => key(13),  // W
        0x58 => key(7),   // X
        0x59 => key(16),  // Y
        0x5A => key(6),   // Z

        // --- Number row ----------------------------------------------------
        0x30 => key(29),  // 0
        0x31 => key(18),  // 1
        0x32 => key(19),  // 2
        0x33 => key(20),  // 3
        0x34 => key(21),  // 4
        0x35 => key(23),  // 5
        0x36 => key(22),  // 6
        0x37 => key(26),  // 7
        0x38 => key(28),  // 8
        0x39 => key(25),  // 9

        // --- Function keys -------------------------------------------------
        0x70 => key(122), // F1
        0x71 => key(120), // F2
        0x72 => key(99),  // F3
        0x73 => key(118), // F4
        0x74 => key(96),  // F5
        0x75 => key(97),  // F6
        0x76 => key(98),  // F7
        0x77 => key(100), // F8
        0x78 => key(101), // F9
        0x79 => key(109), // F10
        0x7A => key(103), // F11
        0x7B => key(111), // F12

        // --- Editing / navigation -----------------------------------------
        0x0D => key(36),  // Enter -> Return
        0x09 => key(48),  // Tab
        0x20 => key(49),  // Space
        0x08 => key(51),  // Backspace -> Delete
        0x1B => key(53),  // Escape
        0x14 => key(57),  // Caps Lock
        0x2E => key(117), // Delete -> Forward Delete
        0x24 => key(115), // Home
        0x23 => key(119), // End
        0x21 => key(116), // Page Up
        0x22 => key(121), // Page Down
        0x25 => key(123), // Left arrow
        0x26 => key(126), // Up arrow
        0x27 => key(124), // Right arrow
        0x28 => key(125), // Down arrow

        // --- Punctuation (US layout OEM keys) -----------------------------
        0xBA => key(41),  // ;
        0xBB => key(24),  // =
        0xBC => key(43),  // ,
        0xBD => key(27),  // -
        0xBE => key(47),  // .
        0xBF => key(44),  // /
        0xC0 => key(50),  // `
        0xDB => key(33),  // [
        0xDC => key(42),  // \
        0xDD => key(30),  // ]
        0xDE => key(39),  // '

        // --- Numpad --------------------------------------------------------
        0x60 => key(82),  // Numpad 0
        0x61 => key(83),  // Numpad 1
        0x62 => key(84),  // Numpad 2
        0x63 => key(85),  // Numpad 3
        0x64 => key(86),  // Numpad 4
        0x65 => key(87),  // Numpad 5
        0x66 => key(88),  // Numpad 6
        0x67 => key(89),  // Numpad 7
        0x68 => key(91),  // Numpad 8
        0x69 => key(92),  // Numpad 9
        0x6A => key(67),  // Numpad *
        0x6B => key(69),  // Numpad +
        0x6D => key(78),  // Numpad -
        0x6E => key(65),  // Numpad .
        0x6F => key(75),  // Numpad /
        0x90 => key(71),  // Num Lock -> Keypad Clear

        // --- Modifiers -----------------------------------------------------
        // Physical default: Ctrl->Control, Win->Command, Alt->Option.
        0x10 | 0xA0 => modifier(56, CGEventFlags::CGEventFlagShift), // (L)Shift
        0xA1 => modifier(60, CGEventFlags::CGEventFlagShift),        // RShift
        0x11 | 0xA2 => {
            if ctrl_as_cmd {
                modifier(55, CGEventFlags::CGEventFlagCommand) // LCtrl -> Cmd
            } else {
                modifier(59, CGEventFlags::CGEventFlagControl) // LCtrl -> Control
            }
        }
        0xA3 => {
            if ctrl_as_cmd {
                modifier(54, CGEventFlags::CGEventFlagCommand) // RCtrl -> RCmd
            } else {
                modifier(62, CGEventFlags::CGEventFlagControl) // RCtrl -> RControl
            }
        }
        0x12 | 0xA4 => modifier(58, CGEventFlags::CGEventFlagAlternate), // (L)Alt -> Option
        0xA5 => modifier(61, CGEventFlags::CGEventFlagAlternate),        // RAlt -> ROption
        0x5B => modifier(55, CGEventFlags::CGEventFlagCommand),          // LWin -> Cmd
        0x5C => modifier(54, CGEventFlags::CGEventFlagCommand),          // RWin -> RCmd

        // PrintScreen, ScrollLock, Pause, IME keys, ... have no macOS
        // equivalent here.
        _ => None,
    }
}
