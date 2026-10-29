//! Modal dialogs: a message box, new-sprite size, open-file list, a text entry with an
//! on-screen keyboard (file and layer names), export options, and the palette color editor.
//! Each dialog knows its widgets and how to paint itself; the app owns the state changes.

use librepixel::doc::{Rgba, Sprite};
use librepixel::gfx::{grey565, Canvas, Rect, BLACK, WHITE};

use crate::ui::{draw_buttons, row, wrap, Id, Widget, BG, DISABLED, SCREEN_H, SCREEN_W, SMALL, TEXT, TITLE};

pub const PANEL_W: i32 = 1200;
pub const MARGIN: i32 = 44;
pub const BUTTON_H: i32 = 84;
pub const OPEN_ROWS: usize = 14;
pub const NAME_MAX: usize = 32;
pub const PRESETS: [i32; 5] = [16, 32, 64, 128, 256];
pub const FORMATS: [&str; 3] = ["PNG", "JPG", "PDF"];
pub const SCALES: [usize; 4] = [1, 2, 4, 8];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Purpose {
    SaveFile,
    RenameLayer,
}

pub enum Dialog {
    Message { title: String, lines: Vec<String>, buttons: Vec<(String, Id)> },
    NewSprite { w: i32, h: i32 },
    Open { files: Vec<String>, selected: Option<usize>, page: usize },
    Text { title: String, name: String, purpose: Purpose },
    Export { format: usize, scale: usize, to_library: bool, name: String },
    EditColor { index: u8, rgb: [u8; 3], drag: Option<usize> },
}

fn panel(h: i32) -> Rect {
    Rect::new((SCREEN_W - PANEL_W) / 2, (SCREEN_H - h) / 2, PANEL_W, h)
}

/// Buttons along the bottom of a panel, right-aligned.
fn bottom_buttons(p: &Rect, items: &[(Id, &str)]) -> Vec<Widget> {
    let w = 240;
    let gap = 16;
    let total = items.len() as i32 * w + (items.len() as i32 - 1) * gap;
    let mut x = p.right() - MARGIN - total;
    let y = p.bottom() - MARGIN - BUTTON_H;
    let mut out = Vec::new();
    for (id, label) in items {
        out.push(Widget::new(*id, Rect::new(x, y, w, BUTTON_H), label));
        x += w + gap;
    }
    out
}

impl Dialog {
    pub fn message(title: &str, text: &str) -> Dialog {
        Dialog::Message { title: title.to_string(), lines: wrap(text, 60), buttons: vec![("OK".to_string(), Id::Ok)] }
    }

    pub fn message_with(title: &str, text: &str, buttons: &[(&str, Id)]) -> Dialog {
        Dialog::Message {
            title: title.to_string(),
            lines: wrap(text, 60),
            buttons: buttons.iter().map(|(l, id)| (l.to_string(), *id)).collect(),
        }
    }

    pub fn rect(&self) -> Rect {
        match self {
            Dialog::Message { lines, .. } => panel(260 + lines.len() as i32 * 36 + BUTTON_H + MARGIN),
            Dialog::NewSprite { .. } => panel(700),
            Dialog::Open { .. } => panel(1560),
            Dialog::Text { .. } => panel(1040),
            Dialog::Export { .. } => panel(820),
            Dialog::EditColor { .. } => panel(900),
        }
    }

    pub fn widgets(&self) -> Vec<Widget> {
        let p = self.rect();
        let x0 = p.x + MARGIN;
        match self {
            Dialog::Message { buttons, .. } => {
                let items: Vec<(Id, &str)> = buttons.iter().map(|(l, id)| (*id, l.as_str())).collect();
                bottom_buttons(&p, &items)
            }
            Dialog::NewSprite { w, h } => {
                let mut ws = Vec::new();
                let y = p.y + 150;
                ws.extend(row(x0 + 260, y, BUTTON_H, 16, &[(Id::SizeW(-8), "-8", 110), (Id::SizeW(-1), "-1", 110)]));
                ws.extend(row(x0 + 700, y, BUTTON_H, 16, &[(Id::SizeW(1), "+1", 110), (Id::SizeW(8), "+8", 110)]));
                let y = p.y + 270;
                ws.extend(row(x0 + 260, y, BUTTON_H, 16, &[(Id::SizeH(-8), "-8", 110), (Id::SizeH(-1), "-1", 110)]));
                ws.extend(row(x0 + 700, y, BUTTON_H, 16, &[(Id::SizeH(1), "+1", 110), (Id::SizeH(8), "+8", 110)]));
                let y = p.y + 420;
                for (i, s) in PRESETS.iter().enumerate() {
                    let label = format!("{s}");
                    ws.push(
                        Widget::new(Id::Preset(*s), Rect::new(x0 + i as i32 * 220, y, 200, BUTTON_H), &label)
                            .active(*w == *s && *h == *s),
                    );
                }
                ws.extend(bottom_buttons(&p, &[(Id::Cancel, "CANCEL"), (Id::Ok, "CREATE")]));
                ws
            }
            Dialog::Open { files, selected, page } => {
                let mut ws = Vec::new();
                let y0 = p.y + 120;
                for i in 0..OPEN_ROWS {
                    let idx = page * OPEN_ROWS + i;
                    if idx >= files.len() {
                        break;
                    }
                    ws.push(
                        Widget::new(Id::Item(idx), Rect::new(x0, y0 + i as i32 * 76, PANEL_W - 2 * MARGIN, 72), &files[idx])
                            .active(*selected == Some(idx))
                            .custom(),
                    );
                }
                let pages = files.len().div_ceil(OPEN_ROWS).max(1);
                let y = p.bottom() - MARGIN - BUTTON_H;
                ws.push(Widget::new(Id::PageUp, Rect::new(x0, y, 160, BUTTON_H), "PREV").enabled(*page > 0));
                ws.push(Widget::new(Id::PageDown, Rect::new(x0 + 176, y, 160, BUTTON_H), "NEXT").enabled(*page + 1 < pages));
                let mut b = bottom_buttons(&p, &[(Id::Cancel, "CANCEL"), (Id::Ok, "OPEN")]);
                b[1].enabled = selected.is_some();
                ws.extend(b);
                ws
            }
            Dialog::Text { .. } => {
                let mut ws = Vec::new();
                let key_w = 104;
                let key_h = 88;
                let gap = 8;
                let rows = ["1234567890", "QWERTYUIOP", "ASDFGHJKL", "ZXCVBNM-_"];
                let y0 = p.y + 300;
                for (r, keys) in rows.iter().enumerate() {
                    let n = keys.chars().count() as i32;
                    let total = n * key_w + (n - 1) * gap;
                    let mut x = p.x + (PANEL_W - total) / 2;
                    for ch in keys.chars() {
                        ws.push(Widget::new(Id::Key(ch), Rect::new(x, y0 + r as i32 * (key_h + gap), key_w, key_h), &ch.to_string()));
                        x += key_w + gap;
                    }
                }
                let y = p.bottom() - MARGIN - BUTTON_H;
                ws.push(Widget::new(Id::Space, Rect::new(x0, y, 340, BUTTON_H), "SPACE"));
                ws.push(Widget::new(Id::Backspace, Rect::new(x0 + 356, y, 240, BUTTON_H), "DELETE"));
                ws.extend(bottom_buttons(&p, &[(Id::Cancel, "CANCEL"), (Id::Ok, "OK")]));
                ws
            }
            Dialog::Export { format, scale, .. } => {
                let mut ws = Vec::new();
                for (i, f) in FORMATS.iter().enumerate() {
                    ws.push(Widget::new(Id::Format(i), Rect::new(x0 + 260 + i as i32 * 200, p.y + 150, 180, BUTTON_H), f).active(*format == i));
                }
                for (i, s) in SCALES.iter().enumerate() {
                    let label = format!("{s}X");
                    ws.push(
                        Widget::new(Id::Scale(i), Rect::new(x0 + 260 + i as i32 * 200, p.y + 280, 180, BUTTON_H), &label)
                            .active(*scale == i)
                            .enabled(*format != 2),
                    );
                }
                ws.extend(bottom_buttons(&p, &[(Id::Cancel, "CANCEL"), (Id::Ok, "EXPORT")]));
                ws
            }
            Dialog::EditColor { .. } => {
                let mut ws = Vec::new();
                for ch in 0..3 {
                    ws.push(Widget::new(Id::Slider(ch), slider_rect(&p, ch), "").custom());
                }
                ws.extend(bottom_buttons(&p, &[(Id::Cancel, "CANCEL"), (Id::Ok, "OK")]));
                ws
            }
        }
    }

    pub fn render(&self, cv: &mut Canvas, sprite: &Sprite) {
        let p = self.rect();
        cv.fill(&p, BG);
        cv.frame(&p, 4, BLACK);
        let x0 = p.x + MARGIN;
        let widgets = self.widgets();
        match self {
            Dialog::Message { title, lines, .. } => {
                cv.text(x0, p.y + MARGIN, TITLE, BLACK, title);
                for (i, l) in lines.iter().enumerate() {
                    cv.text(x0, p.y + 150 + i as i32 * 36, TEXT, BLACK, l);
                }
            }
            Dialog::NewSprite { w, h } => {
                cv.text(x0, p.y + MARGIN, TITLE, BLACK, "NEW SPRITE");
                cv.text(x0, p.y + 150 + 30, TEXT, BLACK, "WIDTH");
                cv.text_in(&Rect::new(x0 + 500, p.y + 150, 180, BUTTON_H), TITLE, BLACK, &format!("{w}"));
                cv.text(x0, p.y + 270 + 30, TEXT, BLACK, "HEIGHT");
                cv.text_in(&Rect::new(x0 + 500, p.y + 270, 180, BUTTON_H), TITLE, BLACK, &format!("{h}"));
                cv.text(x0, p.y + 390, SMALL, BLACK, "SQUARE PRESETS");
            }
            Dialog::Open { files, .. } => {
                cv.text(x0, p.y + MARGIN, TITLE, BLACK, "OPEN");
                if files.is_empty() {
                    cv.text(x0, p.y + 150, TEXT, BLACK, "NO SPRITES YET. SAVE ONE FIRST,");
                    cv.text(x0, p.y + 190, TEXT, BLACK, "OR COPY .ASE OR .PNG FILES INTO THE SPRITES FOLDER.");
                }
                for w in widgets.iter().filter(|w| !w.plain) {
                    let (fill, ink) = if w.active { (BLACK, WHITE) } else { (WHITE, BLACK) };
                    cv.fill(&w.rect, fill);
                    cv.frame(&w.rect, 2, BLACK);
                    cv.text_left(&w.rect, 20, TEXT, ink, &w.label);
                }
            }
            Dialog::Text { title, name, .. } => {
                cv.text(x0, p.y + MARGIN, TITLE, BLACK, title);
                let field = Rect::new(x0, p.y + 150, PANEL_W - 2 * MARGIN, 100);
                cv.fill(&field, WHITE);
                cv.frame(&field, 3, BLACK);
                cv.text_left(&field, 20, TITLE, BLACK, &format!("{name}_"));
            }
            Dialog::Export { format, to_library, name, .. } => {
                cv.text(x0, p.y + MARGIN, TITLE, BLACK, "EXPORT");
                cv.text(x0, p.y + 150 + 30, TEXT, BLACK, "FORMAT");
                cv.text(x0, p.y + 280 + 30, TEXT, BLACK, "SCALE");
                let info = match (*format, *to_library) {
                    (2, true) => "PDF GOES INTO THE TABLET'S OWN LIBRARY, NEXT TO YOUR NOTEBOOKS.",
                    (2, false) => "PDF IS WRITTEN NEXT TO THE SPRITE FILES.",
                    _ => "WRITTEN NEXT TO THE SPRITE FILES. COPY IT OFF WITH USB OR SSH.",
                };
                for (i, l) in wrap(info, 60).iter().enumerate() {
                    cv.text(x0, p.y + 420 + i as i32 * 36, TEXT, BLACK, l);
                }
                cv.text(x0, p.y + 520, TEXT, BLACK, &format!("NAME: {name}.{}", FORMATS[*format].to_lowercase()));
            }
            Dialog::EditColor { index, rgb, .. } => {
                cv.text(x0, p.y + MARGIN, TITLE, BLACK, &format!("COLOR {index}"));
                let names = ["RED", "GREEN", "BLUE"];
                for ch in 0..3 {
                    let r = slider_rect(&p, ch);
                    cv.text(x0, r.y + 24, TEXT, BLACK, names[ch]);
                    cv.fill(&r, WHITE);
                    let filled = Rect::new(r.x, r.y, r.w * rgb[ch] as i32 / 255, r.h);
                    cv.fill(&filled, grey565(96));
                    cv.frame(&r, 3, BLACK);
                    cv.text(r.right() + 20, r.y + 24, TEXT, BLACK, &format!("{:>3}", rgb[ch]));
                }
                // What the panel will show for it, and the true color as numbers.
                let c = Rgba::rgb(rgb[0], rgb[1], rgb[2]);
                let swatch = Rect::new(x0, p.y + 560, 160, 160);
                cv.fill(&swatch, grey565(c.luma()));
                cv.frame(&swatch, 3, BLACK);
                cv.text(x0 + 190, p.y + 580, TEXT, BLACK, "TRUE COLOR, SHOWN AS ITS BRIGHTNESS HERE.");
                cv.text(x0 + 190, p.y + 620, TEXT, BLACK, "FILES KEEP THE RGB VALUES.");
                let _ = sprite;
                let _ = DISABLED;
            }
        }
        draw_buttons(cv, &widgets);
    }
}

pub fn slider_rect(p: &Rect, ch: usize) -> Rect {
    Rect::new(p.x + MARGIN + 160, p.y + 150 + ch as i32 * 120, 760, 72)
}
