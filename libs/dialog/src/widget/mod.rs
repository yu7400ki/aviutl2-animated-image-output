mod button;
mod checkbox;
mod combobox;
mod label;
mod number;
mod textbox;

pub use button::*;
pub use checkbox::*;
pub use combobox::*;
pub use label::*;
pub use number::*;
pub use textbox::*;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HFONT;

use crate::{ControlId, Result};

pub trait Widget {
    fn get_id(&self) -> ControlId;
    fn handle_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT>;
    fn get_hwnd(&self) -> Option<HWND>;
    fn create_node(
        &self,
        tree: &mut taffy::TaffyTree,
        font: Option<HFONT>,
    ) -> Result<taffy::NodeId>;
    fn create_window(
        &mut self,
        parent: HWND,
        _taffy: &taffy::TaffyTree,
        position: (i32, i32),
    ) -> Result<()>;
    fn set_font(&self, font: HFONT);
}
