use crossterm::event::{KeyCode, KeyEvent};

use crate::app::Message;

pub fn key_to_message(key: KeyEvent) -> Option<Message> {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Some(Message::Quit),

        KeyCode::Right | KeyCode::Char('l' | ' ') | KeyCode::PageDown => {
            Some(Message::NextPage)
        }
        KeyCode::Left | KeyCode::Char('h') | KeyCode::PageUp => Some(Message::PrevPage),

        KeyCode::Char('g') | KeyCode::Home => Some(Message::FirstPage),
        KeyCode::Char('G') | KeyCode::End => Some(Message::LastPage),

        KeyCode::Char('+' | '=') => Some(Message::ZoomIn),
        KeyCode::Char('-') => Some(Message::ZoomOut),
        KeyCode::Char('0') => Some(Message::ZoomReset),

        KeyCode::Up | KeyCode::Char('k') => Some(Message::ScrollUp),
        KeyCode::Down | KeyCode::Char('j') => Some(Message::ScrollDown),
        KeyCode::Char('H') => Some(Message::ScrollLeft),
        KeyCode::Char('L') => Some(Message::ScrollRight),

        KeyCode::Char('d') => Some(Message::CycleLayout),
        KeyCode::Char('n') => Some(Message::ToggleDarkMode),
        KeyCode::Char('f') => Some(Message::ToggleFullscreen),
        KeyCode::Char('p') => Some(Message::EnterGoto),

        _ => None,
    }
}

pub fn key_to_goto_message(key: KeyEvent) -> Option<Message> {
    match key.code {
        KeyCode::Char(c) if c.is_ascii_digit() => Some(Message::GotoInput(c)),
        KeyCode::Backspace => Some(Message::GotoBackspace),
        KeyCode::Enter => Some(Message::GotoConfirm),
        KeyCode::Esc => Some(Message::GotoCancel),
        _ => None,
    }
}
