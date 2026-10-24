//! Zrzut ekranu z adnotacją: zdjęcie monitora pod kursorem, w oknie na całym
//! ekranie zaznaczasz obszar, dorysowujesz strzałkę / ramkę / pisak, Enter
//! zapisuje do Obrazy\Zrzuty ekranu, kopiuje do schowka i kładzie na półce.
//! Do tego szybki podgląd plików (miniatura z Windows albo tekst).

use crate::{commands, platform};
use base64::Engine;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ShotData {
    image: String,
    width: u32,
    height: u32,
}

/// Robi zdjęcie monitora pod kursorem i otwiera okno zaznaczania.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let (cx, cy) = platform::cursor().unwrap_or((0, 0));
        let mon = app.available_monitors().unwrap_or_default().into_iter().find(|m| {
            let p = m.position();
            let z = m.size();
            cx >= p.x && cx < p.x + z.width as i32 && cy >= p.y && cy < p.y + z.height as i32
        });
        let Some(m) = mon.or_else(|| app.primary_monitor().ok().flatten()) else { return };
        let (x, y, w, h) = (m.position().x, m.position().y, m.size().width, m.size().height);
        let Some(png) = imp::capture(x, y, w, h) else { return };
        let data = ShotData {
            image: format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)),
            width: w,
            height: h,
        };
        let Some(win) = app.get_webview_window("shot") else { return };
        let _ = win.set_position(PhysicalPosition::new(x, y));
        let _ = win.set_size(PhysicalSize::new(w, h));
        let _ = app.emit_to("shot", "shot", data);
        let _ = win.show();
        let _ = win.set_focus();
    });
}

#[tauri::command]
pub fn shot_start(app: AppHandle) {
    start(&app);
}

#[tauri::command]
pub fn shot_cancel(app: AppHandle) {
    if let Some(w) = app.get_webview_window("shot") {
        let _ = w.hide();
    }
}

/// Zapisuje wynik (PNG jako data URL): plik + schowek + półka.
#[tauri::command]
pub async fn shot_save(app: AppHandle, png: String) -> Result<String, String> {
    shot_cancel(app.clone());
    let b64 = png.split_once(',').map(|(_, d)| d).unwrap_or(&png);
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| e.to_string())?;
    let dir = dirs::picture_dir().unwrap_or_else(|| std::path::PathBuf::from(".")).join("Screenshots");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("Zrzut {}.png", chrono::Local::now().format("%Y-%m-%d %H-%M-%S")));
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    // Schowek: obraz RGBA z PNG.
    if let Ok((rgba, w, h)) = decode_png(&bytes) {
        use tauri_plugin_clipboard_manager::ClipboardExt;
        let img = tauri::image::Image::new_owned(rgba, w, h);
        let _ = app.clipboard().write_image(&img);
    }
    let p = path.to_string_lossy().to_string();
    commands::shelf_add(app.clone(), vec![p.clone()]);
    Ok(p)
}

fn decode_png(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    let dec = png::Decoder::new(bytes);
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (w, h) = (info.width, info.height);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..(w * h * 4) as usize].to_vec(),
        png::ColorType::Rgb => buf[..(w * h * 3) as usize].chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        _ => return Err("nieobsługiwany format".into()),
    };
    Ok((rgba, w, h))
}

// ---------------------------------------------------------------------------
// Szybki podgląd plików
// ---------------------------------------------------------------------------

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    name: String,
    path: String,
    kind: String,
    image: Option<String>,
    text: Option<String>,
    info: String,
}

fn human(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{} KB", b / 1024),
        b => format!("{b} B"),
    }
}

pub fn preview_of(path: &str) -> Preview {
    let p = std::path::Path::new(path);
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let md = std::fs::metadata(p).ok();
    let modified = md
        .as_ref()
        .and_then(|m| m.modified().ok())
        .map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%d.%m.%Y %H:%M").to_string())
        .unwrap_or_default();
    let info = format!("{} · zmieniony {modified}", md.as_ref().map(|m| if m.is_dir() { "folder".to_string() } else { human(m.len()) }).unwrap_or_default());
    let b64 = |b: &[u8], mime: &str| format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(b));
    // Obrazy — w oryginale (do 20 MB).
    let mime = match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    };
    if let (Some(mime), Some(m)) = (mime, &md) {
        if m.len() <= 20 * 1024 * 1024 {
            if let Ok(bytes) = std::fs::read(p) {
                return Preview { name, path: path.into(), kind: "image".into(), image: Some(b64(&bytes, mime)), text: None, info };
            }
        }
    }
    // Tekst — pierwsze 64 KB.
    let texty = matches!(
        ext.as_str(),
        "txt" | "md" | "csv" | "log" | "json" | "xml" | "yml" | "yaml" | "toml" | "ini" | "cfg" | "py" | "rs" | "js" | "ts" | "html" | "css" | "sql" | "ps1" | "bat" | "sh"
    );
    if texty {
        if let Ok(bytes) = std::fs::read(p) {
            let cut = &bytes[..bytes.len().min(64 * 1024)];
            return Preview { name, path: path.into(), kind: "text".into(), image: None, text: Some(String::from_utf8_lossy(cut).to_string()), info };
        }
    }
    // Reszta (PDF, Office, wideo, foldery) — miniatura z Eksploratora.
    let image = imp::thumbnail(path, 1200).map(|png| b64(&png, "image/png"));
    Preview { name, path: path.into(), kind: if image.is_some() { "thumb".into() } else { "none".into() }, image, text: None, info }
}

#[tauri::command]
pub async fn quicklook(app: AppHandle, path: String) -> Result<(), String> {
    let pv = tauri::async_runtime::spawn_blocking(move || preview_of(&path)).await.map_err(|e| e.to_string())?;
    let Some(w) = app.get_webview_window("quicklook") else { return Err("brak okna".into()) };
    let s = crate::layout::screen(&app);
    let (pw, ph) = ((960.0 * s.scale) as i32, (720.0 * s.scale) as i32);
    let _ = w.set_size(PhysicalSize::new(pw as u32, ph as u32));
    let _ = w.set_position(PhysicalPosition::new((s.work.left + s.work.right - pw) / 2, (s.work.top + s.work.bottom - ph) / 2));
    let _ = app.emit_to("quicklook", "quicklook", pv);
    let _ = w.show();
    let _ = w.set_focus();
    Ok(())
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use windows::core::HSTRING;
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC,
        SelectObject, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HBITMAP, HDC, SRCCOPY,
    };
    use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_RESIZETOFIT};

    fn encode(px: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Fast);
            let mut wr = enc.write_header().ok()?;
            wr.write_image_data(px).ok()?;
        }
        Some(out)
    }

    /// Bitmapa → PNG (BGRA → RGBA; brak alfy = nieprzezroczysta).
    unsafe fn bitmap_png(bmp: HBITMAP, w: i32, h: i32) -> Option<Vec<u8>> {
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut px = vec![0u8; (w * h * 4) as usize];
        let dc = CreateCompatibleDC(HDC::default());
        let lines = GetDIBits(dc, bmp, 0, h as u32, Some(px.as_mut_ptr() as *mut c_void), &mut bmi, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        if lines == 0 {
            return None;
        }
        let has_alpha = px.chunks(4).any(|p| p[3] != 0);
        for p in px.chunks_mut(4) {
            p.swap(0, 2);
            if !has_alpha {
                p[3] = 255;
            }
        }
        encode(&px, w as u32, h as u32)
    }

    pub fn capture(x: i32, y: i32, w: u32, h: u32) -> Option<Vec<u8>> {
        unsafe {
            let screen = GetDC(windows::Win32::Foundation::HWND::default());
            let mem = CreateCompatibleDC(screen);
            let bmp = CreateCompatibleBitmap(screen, w as i32, h as i32);
            let old = SelectObject(mem, bmp);
            let ok = BitBlt(mem, 0, 0, w as i32, h as i32, screen, x, y, SRCCOPY | CAPTUREBLT).is_ok();
            SelectObject(mem, old);
            let png = if ok { bitmap_png(bmp, w as i32, h as i32) } else { None };
            let _ = DeleteObject(bmp);
            let _ = DeleteDC(mem);
            ReleaseDC(windows::Win32::Foundation::HWND::default(), screen);
            // Zrzut nie ma alfy — bitmap_png ustawia 255, gdy kanał jest pusty.
            png
        }
    }

    /// Miniatura pliku taka, jak w Eksploratorze (PDF, Office, wideo…).
    pub fn thumbnail(path: &str, size: i32) -> Option<Vec<u8>> {
        unsafe {
            let item: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
            let bmp = item.GetImage(SIZE { cx: size, cy: size }, SIIGBF_BIGGERSIZEOK | SIIGBF_RESIZETOFIT).ok()?;
            let mut bm = BITMAP::default();
            GetObjectW(bmp, std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut BITMAP as *mut c_void));
            let out = if bm.bmWidth > 0 && bm.bmHeight > 0 { bitmap_png(bmp, bm.bmWidth, bm.bmHeight) } else { None };
            let _ = DeleteObject(bmp);
            out
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn capture(_: i32, _: i32, _: u32, _: u32) -> Option<Vec<u8>> { None }
    pub fn thumbnail(_: &str, _: i32) -> Option<Vec<u8>> { None }
}
