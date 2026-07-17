use crate::{DialogError, Result};
use std::rc::Rc;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::*;
use windows::core::{HSTRING, w};

struct FontInner {
    hfont: HFONT,
    owned: bool,
}

impl Drop for FontInner {
    fn drop(&mut self) {
        if self.owned && !self.hfont.is_invalid() {
            unsafe {
                let _ = DeleteObject(self.hfont.into());
            }
        }
    }
}

/// ダイアログで使用するフォント。
///
/// クローンは同じ`HFONT`を共有し、最後のクローンが破棄されたときに一度だけ
/// `HFONT`を解放する(`system`で作成した場合のみ)。
#[derive(Clone)]
pub struct Font(Rc<FontInner>);

impl Font {
    /// 指定DPIに合わせた既定フォント(Meiryo UI)を作成する
    pub fn system(dpi: u32) -> Result<Self> {
        let height = -(12 * dpi as i32).div_euclid(96);
        let hfont = unsafe {
            CreateFontW(
                height,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                DEFAULT_QUALITY,
                (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
                w!("Meiryo UI"),
            )
        };
        if hfont.is_invalid() {
            Err(DialogError::Win32Error(windows::core::Error::from_thread()))
        } else {
            Ok(Font(Rc::new(FontInner { hfont, owned: true })))
        }
    }

    /// 呼び出し側が寿命を管理する`HFONT`をラップする(このFontからは解放しない)
    pub fn from_hfont(hfont: HFONT) -> Self {
        Font(Rc::new(FontInner {
            hfont,
            owned: false,
        }))
    }

    pub fn hfont(&self) -> HFONT {
        self.0.hfont
    }

    /// テキストの物理ピクセルサイズを測定する
    pub fn measure(&self, text: &str) -> Result<(i32, i32)> {
        unsafe {
            let hdc = GetDC(None);
            if hdc.is_invalid() {
                return Err(DialogError::Win32Error(windows::core::Error::from_thread()));
            }
            // 共有DCを汚さないよう、選択したフォントは必ず元に戻してから返却する
            let prev = SelectObject(hdc, self.0.hfont.into());
            let mut size = SIZE::default();
            let text_wide = HSTRING::from(text);
            let ok = GetTextExtentPoint32W(hdc, &text_wide, &mut size);
            if !prev.is_invalid() {
                SelectObject(hdc, prev);
            }
            ReleaseDC(None, hdc);
            if ok.as_bool() {
                Ok((size.cx, size.cy))
            } else {
                Err(DialogError::Win32Error(windows::core::Error::from_thread()))
            }
        }
    }
}
