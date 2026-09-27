//! The Wayland side of slate-desktop: one connection that owns a transient
//! seat (the agent's own pointer and keyboard), tracks toplevels, and captures
//! windows or outputs through ext-image-copy-capture.
//!
//! Everything here runs on one thread; callers use [`Desktop`] directly.

use crate::keymap::{self, Keymap};
use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::time::{Duration, Instant};
use wayland_client::globals::{registry_queue_init, GlobalList, GlobalListContents};
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_seat, wl_shm, wl_shm_pool};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1 as handle, ext_foreign_toplevel_list_v1 as list,
};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_foreign_toplevel_image_capture_source_manager_v1 as tl_source_mgr,
    ext_image_capture_source_v1 as source,
    ext_output_image_capture_source_manager_v1 as out_source_mgr,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1 as frame, ext_image_copy_capture_manager_v1 as capture_mgr,
    ext_image_copy_capture_session_v1 as session,
};
use wayland_protocols::ext::transient_seat::v1::client::{
    ext_transient_seat_manager_v1 as tseat_mgr, ext_transient_seat_v1 as tseat,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1 as vk_mgr, zwp_virtual_keyboard_v1 as vk,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1 as vp_mgr, zwlr_virtual_pointer_v1 as vp,
};

/// Which seat an input action uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Seat {
    /// The agent's own transient seat. The human keeps their mouse and keyboard.
    #[default]
    Agent,
    /// The human's seat (seat0). Needed for toolkits that only bind the first
    /// seat (GTK4); takes over the user's input while the action runs.
    User,
}

impl Seat {
    pub fn parse(s: &str) -> Self {
        if s.eq_ignore_ascii_case("user") {
            Seat::User
        } else {
            Seat::Agent
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Toplevel {
    pub identifier: String,
    pub app_id: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct Output {
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub scale: i32,
    pub proxy: wl_output::WlOutput,
}

#[derive(Debug, Default)]
struct CaptureState {
    width: u32,
    height: u32,
    format: Option<wl_shm::Format>,
    session_done: bool,
    frame_ready: bool,
    frame_failed: Option<String>,
}

#[derive(Default)]
pub struct State {
    toplevels: Vec<(handle::ExtForeignToplevelHandleV1, Toplevel)>,
    pending: HashMap<u32, Toplevel>, // by proxy id, until `done`
    outputs: Vec<Output>,
    pending_outputs: HashMap<u32, Output>,
    transient_ready: Option<u32>,
    capture: CaptureState,
}

pub struct Desktop {
    _conn: Connection,
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    _globals: GlobalList,
    pub state: State,
    shm: wl_shm::WlShm,
    capture_mgr: capture_mgr::ExtImageCopyCaptureManagerV1,
    out_source_mgr: out_source_mgr::ExtOutputImageCaptureSourceManagerV1,
    tl_source_mgr: Option<tl_source_mgr::ExtForeignToplevelImageCaptureSourceManagerV1>,
    _toplevel_list: list::ExtForeignToplevelListV1,
    _transient: tseat::ExtTransientSeatV1,
    pub seat_name: Option<u32>,
    pointer: vp::ZwlrVirtualPointerV1,
    keyboard: vk::ZwpVirtualKeyboardV1,
    /// Virtual devices on the user's own seat, for the GTK4 fallback.
    user_pointer: vp::ZwlrVirtualPointerV1,
    user_keyboard: vk::ZwpVirtualKeyboardV1,
    keymap: Option<Keymap>,
    /// Keymap uploaded to the user-seat keyboard (tracked separately).
    user_keymap: Option<Keymap>,
    started: Instant,
}

impl Desktop {
    pub fn connect() -> Result<Self> {
        let conn = Connection::connect_to_env()
            .context("connecting to the Wayland display (is WAYLAND_DISPLAY set?)")?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
        let qh = queue.handle();
        let mut state = State::default();

        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).context("wl_shm")?;
        let capture_mgr: capture_mgr::ExtImageCopyCaptureManagerV1 =
            globals
                .bind(&qh, 1..=1, ())
                .context("ext_image_copy_capture_manager_v1 (compositor too old?)")?;
        let out_source_mgr: out_source_mgr::ExtOutputImageCaptureSourceManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .context("ext_output_image_capture_source_manager_v1")?;
        let tl_source_mgr = globals.bind(&qh, 1..=1, ()).ok();
        let toplevel_list: list::ExtForeignToplevelListV1 = globals
            .bind(&qh, 1..=1, ())
            .context("ext_foreign_toplevel_list_v1")?;
        let tseat_mgr: tseat_mgr::ExtTransientSeatManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .context("ext_transient_seat_manager_v1 (compositor must support transient seats)")?;
        let vp_mgr: vp_mgr::ZwlrVirtualPointerManagerV1 = globals
            .bind(&qh, 2..=2, ())
            .context("zwlr_virtual_pointer_manager_v1 v2")?;
        let vk_mgr: vk_mgr::ZwpVirtualKeyboardManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .context("zwp_virtual_keyboard_manager_v1")?;

        // Outputs.
        globals.contents().with_list(|list| {
            for g in list {
                if g.interface == wl_output::WlOutput::interface().name {
                    let out: wl_output::WlOutput =
                        globals.registry().bind(g.name, 4.min(g.version), &qh, ());
                    state.pending_outputs.insert(
                        out.id().protocol_id(),
                        Output {
                            name: String::new(),
                            width: 0,
                            height: 0,
                            scale: 1,
                            proxy: out,
                        },
                    );
                }
            }
        });

        let transient = tseat_mgr.create(&qh, ());
        queue.roundtrip(&mut state)?;
        queue.roundtrip(&mut state)?;
        let seat_name = state
            .transient_ready
            .ok_or_else(|| anyhow!("compositor did not grant a transient seat"))?;
        let seat: wl_seat::WlSeat = globals.registry().bind(seat_name, 7, &qh, ());
        let output = state.outputs.first().map(|o| o.proxy.clone());
        let pointer =
            vp_mgr.create_virtual_pointer_with_output(Some(&seat), output.as_ref(), &qh, ());
        let keyboard = vk_mgr.create_virtual_keyboard(&seat, &qh, ());
        // The user's seat: the first wl_seat global that is not ours.
        let mut user_seat_name = None;
        globals.contents().with_list(|list| {
            for g in list {
                if g.interface == wl_seat::WlSeat::interface().name && g.name != seat_name {
                    user_seat_name = Some(g.name);
                    break;
                }
            }
        });
        let user_seat: wl_seat::WlSeat = match user_seat_name {
            Some(n) => globals.registry().bind(n, 7, &qh, ()),
            None => seat.clone(),
        };
        let user_pointer =
            vp_mgr.create_virtual_pointer_with_output(Some(&user_seat), output.as_ref(), &qh, ());
        let user_keyboard = vk_mgr.create_virtual_keyboard(&user_seat, &qh, ());
        queue.roundtrip(&mut state)?;

        let mut d = Self {
            _conn: conn,
            queue,
            qh,
            _globals: globals,
            state,
            shm,
            capture_mgr,
            out_source_mgr,
            tl_source_mgr,
            _toplevel_list: toplevel_list,
            _transient: transient,
            seat_name: Some(seat_name),
            pointer,
            keyboard,
            user_pointer,
            user_keyboard,
            keymap: None,
            user_keymap: None,
            started: Instant::now(),
        };
        // Upload a baseline keymap so key combos work before any text is typed.
        d.ensure_keymap(Seat::Agent, "")?;
        Ok(d)
    }

    pub fn roundtrip(&mut self) -> Result<()> {
        self.queue.roundtrip(&mut self.state)?;
        Ok(())
    }

    fn now_ms(&self) -> u32 {
        self.started.elapsed().as_millis() as u32
    }

    fn ptr(&self, seat: Seat) -> &vp::ZwlrVirtualPointerV1 {
        match seat {
            Seat::Agent => &self.pointer,
            Seat::User => &self.user_pointer,
        }
    }

    fn kbd(&self, seat: Seat) -> &vk::ZwpVirtualKeyboardV1 {
        match seat {
            Seat::Agent => &self.keyboard,
            Seat::User => &self.user_keyboard,
        }
    }

    fn keymap_slot(&mut self, seat: Seat) -> &mut Option<Keymap> {
        match seat {
            Seat::Agent => &mut self.keymap,
            Seat::User => &mut self.user_keymap,
        }
    }

    pub fn toplevels(&mut self) -> Result<Vec<Toplevel>> {
        self.roundtrip()?;
        Ok(self
            .state
            .toplevels
            .iter()
            .map(|(_, t)| t.clone())
            .collect())
    }

    pub fn outputs(&self) -> &[Output] {
        &self.state.outputs
    }

    fn find_handle(&self, ident: &str) -> Option<handle::ExtForeignToplevelHandleV1> {
        self.state
            .toplevels
            .iter()
            .find(|(_, t)| t.identifier == ident)
            .map(|(h, _)| h.clone())
    }

    // ---- input ---------------------------------------------------------

    /// Move the agent pointer to absolute output coordinates.
    pub fn pointer_move(&mut self, seat: Seat, x: f64, y: f64) -> Result<()> {
        let (w, h) = self.extent()?;
        let t = self.now_ms();
        let p = self.ptr(seat);
        p.motion_absolute(t, x.max(0.0) as u32, y.max(0.0) as u32, w as u32, h as u32);
        p.frame();
        self.queue.flush()?;
        self.roundtrip()
    }

    /// Click at absolute coordinates. `button`: "left" | "right" | "middle".
    pub fn click(&mut self, seat: Seat, x: f64, y: f64, button: &str, count: u32) -> Result<()> {
        self.pointer_move(seat, x, y)?;
        let code = match button {
            "right" => 0x111,
            "middle" => 0x112,
            _ => 0x110,
        };
        for _ in 0..count.max(1) {
            let t = self.now_ms();
            self.pointer.button(
                t,
                code,
                wayland_client::protocol::wl_pointer::ButtonState::Pressed,
            );
            self.pointer.frame();
            let t = self.now_ms();
            self.pointer.button(
                t,
                code,
                wayland_client::protocol::wl_pointer::ButtonState::Released,
            );
            self.pointer.frame();
            self.queue.flush()?;
            std::thread::sleep(Duration::from_millis(40));
        }
        self.roundtrip()
    }

    pub fn scroll(&mut self, seat: Seat, x: f64, y: f64, dx: f64, dy: f64) -> Result<()> {
        use wayland_client::protocol::wl_pointer::Axis;
        self.pointer_move(seat, x, y)?;
        let t = self.now_ms();
        let p = self.ptr(seat);
        if dy != 0.0 {
            p.axis(t, Axis::VerticalScroll, dy);
        }
        if dx != 0.0 {
            p.axis(t, Axis::HorizontalScroll, dx);
        }
        p.frame();
        self.queue.flush()?;
        self.roundtrip()
    }

    fn ensure_keymap(&mut self, seat: Seat, text: &str) -> Result<()> {
        let needs = match self.keymap_slot(seat) {
            None => true,
            Some(k) => text
                .chars()
                .any(|c| k.code_for_char(c).is_none() && !c.is_control()),
        };
        if !needs {
            return Ok(());
        }
        // Build a keymap that covers the new text plus everything typed so far.
        let combined = match self.keymap_slot(seat) {
            Some(k) => {
                let mut s = k.text_chars();
                s.push_str(text);
                s
            }
            None => text.to_string(),
        };
        let km = Keymap::for_text(&combined);
        let fd = memfd("slate-keymap", km.text.as_bytes())?;
        self.kbd(seat)
            .keymap(1, fd.as_fd(), km.text.len() as u32 + 1);
        self.queue.flush()?;
        *self.keymap_slot(seat) = Some(km);
        self.roundtrip()?;
        // Absorb the one key event that gets lost after a keymap change.
        self.key_event(seat, keymap::VOID_CODE, true);
        self.key_event(seat, keymap::VOID_CODE, false);
        self.queue.flush()?;
        std::thread::sleep(Duration::from_millis(20));
        self.roundtrip()
    }

    fn key_event(&mut self, seat: Seat, code: u32, pressed: bool) {
        let t = self.now_ms();
        self.kbd(seat).key(t, code, if pressed { 1 } else { 0 });
    }

    /// Type text on the agent keyboard.
    pub fn type_text(&mut self, seat: Seat, text: &str) -> Result<()> {
        self.ensure_keymap(seat, text)?;
        let codes: Vec<u32> = {
            let km = self.keymap_slot(seat).as_ref();
            text.chars()
                .filter_map(|c| km.and_then(|k| k.code_for_char(c)))
                .collect()
        };
        for code in codes {
            self.key_event(seat, code, true);
            self.key_event(seat, code, false);
            self.queue.flush()?;
            std::thread::sleep(Duration::from_millis(6));
        }
        self.roundtrip()
    }

    /// Press a key combo like "ctrl+l", "Return", "alt+Tab", "shift+a".
    pub fn key(&mut self, seat: Seat, combo: &str) -> Result<()> {
        let (mods, key) = keymap::parse_combo(combo);
        if key.is_empty() {
            bail!("empty key");
        }
        // Single character keys go through the char table.
        let is_char = key.chars().count() == 1;
        if is_char {
            self.ensure_keymap(seat, &key)?;
        } else {
            self.ensure_keymap(seat, "")?;
        }
        let km = self.keymap_slot(seat).as_ref().unwrap();
        let key_code = if is_char {
            km.code_for_char(key.chars().next().unwrap())
        } else {
            km.code_for_named(&key)
        }
        .ok_or_else(|| anyhow!("unknown key {key:?}"))?;
        let mut mod_codes = vec![];
        let mut mask = 0u32;
        for m in &mods {
            let c = km
                .code_for_named(m)
                .ok_or_else(|| anyhow!("unknown modifier {m:?}"))?;
            mask |= keymap::modifier_mask(m).unwrap_or(0);
            mod_codes.push(c);
        }
        for c in &mod_codes {
            self.key_event(seat, *c, true);
        }
        if mask != 0 {
            self.kbd(seat).modifiers(mask, 0, 0, 0);
        }
        self.key_event(seat, key_code, true);
        self.key_event(seat, key_code, false);
        if mask != 0 {
            self.kbd(seat).modifiers(0, 0, 0, 0);
        }
        for c in mod_codes.iter().rev() {
            self.key_event(seat, *c, false);
        }
        self.queue.flush()?;
        self.roundtrip()
    }

    fn extent(&self) -> Result<(i32, i32)> {
        let o = self
            .state
            .outputs
            .first()
            .ok_or_else(|| anyhow!("no outputs"))?;
        Ok((o.width.max(1), o.height.max(1)))
    }

    // ---- capture -------------------------------------------------------

    /// Capture a toplevel (by identifier) or, with `None`, the first output. Returns PNG bytes and size.
    /// `crop_to` trims the capture to the given content size, centred: toolkits with client-side
    /// decorations (GTK, Firefox) render invisible shadow margins around the window, and the
    /// compositor reports the window geometry without them. Cropping keeps screenshot pixels
    /// aligned with window-relative click coordinates.
    pub fn capture(
        &mut self,
        ident: Option<&str>,
        crop_to: Option<(u32, u32)>,
    ) -> Result<(Vec<u8>, u32, u32)> {
        self.roundtrip()?;
        let src: source::ExtImageCaptureSourceV1 = match ident {
            Some(id) => {
                let h = self
                    .find_handle(id)
                    .ok_or_else(|| anyhow!("no window with identifier {id}"))?;
                let mgr = self
                    .tl_source_mgr
                    .as_ref()
                    .ok_or_else(|| anyhow!("compositor cannot capture individual windows"))?;
                mgr.create_source(&h, &self.qh, ())
            }
            None => {
                let o = self
                    .state
                    .outputs
                    .first()
                    .ok_or_else(|| anyhow!("no outputs"))?
                    .proxy
                    .clone();
                self.out_source_mgr.create_source(&o, &self.qh, ())
            }
        };
        self.state.capture = CaptureState::default();
        let sess =
            self.capture_mgr
                .create_session(&src, capture_mgr::Options::empty(), &self.qh, ());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.state.capture.session_done {
            if Instant::now() > deadline {
                bail!("timed out waiting for capture session");
            }
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        let (w, h) = (self.state.capture.width, self.state.capture.height);
        if w == 0 || h == 0 {
            sess.destroy();
            src.destroy();
            bail!("capture session reported an empty size");
        }
        let format = self
            .state
            .capture
            .format
            .unwrap_or(wl_shm::Format::Xrgb8888);
        let stride = w * 4;
        let size = (stride * h) as usize;
        let fd = memfd("slate-capture", &vec![0u8; size])?;
        let pool = self.shm.create_pool(fd.as_fd(), size as i32, &self.qh, ());
        let buffer = pool.create_buffer(0, w as i32, h as i32, stride as i32, format, &self.qh, ());
        let fr = sess.create_frame(&self.qh, ());
        fr.attach_buffer(&buffer);
        fr.damage_buffer(0, 0, w as i32, h as i32);
        fr.capture();
        self.queue.flush()?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.state.capture.frame_ready && self.state.capture.frame_failed.is_none() {
            if Instant::now() > deadline {
                fr.destroy();
                buffer.destroy();
                pool.destroy();
                sess.destroy();
                src.destroy();
                bail!("timed out waiting for the frame");
            }
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        let failed = self.state.capture.frame_failed.clone();
        fr.destroy();
        buffer.destroy();
        pool.destroy();
        sess.destroy();
        src.destroy();
        self.queue.flush()?;
        if let Some(reason) = failed {
            bail!("capture failed: {reason}");
        }
        let mut file = std::fs::File::from(fd);
        file.seek(SeekFrom::Start(0))?;
        let mut raw = vec![0u8; size];
        file.read_exact(&mut raw)?;
        let (raw, w, h) = match crop_to {
            Some((cw, ch)) if cw > 0 && ch > 0 && (cw < w || ch < h) => {
                let cw = cw.min(w);
                let ch = ch.min(h);
                let ox = (w - cw) / 2;
                let oy = (h - ch) / 2;
                let mut out = Vec::with_capacity((cw * ch * 4) as usize);
                for row in oy..oy + ch {
                    let start = ((row * w + ox) * 4) as usize;
                    out.extend_from_slice(&raw[start..start + (cw * 4) as usize]);
                }
                (out, cw, ch)
            }
            _ => (raw, w, h),
        };
        let png = encode_png(&raw, w, h, format)?;
        Ok((png, w, h))
    }
}

/// An anonymous shared-memory file (memfd on Linux, an unlinked temp file elsewhere).
fn memfd(name: &str, contents: &[u8]) -> Result<OwnedFd> {
    #[cfg(target_os = "linux")]
    let mut f = {
        use std::os::fd::FromRawFd;
        let cname = std::ffi::CString::new(name)?;
        let raw = unsafe { libc::memfd_create(cname.as_ptr(), libc::MFD_CLOEXEC) };
        if raw < 0 {
            return Err(std::io::Error::last_os_error()).context("memfd_create");
        }
        unsafe { std::fs::File::from_raw_fd(raw) }
    };
    #[cfg(not(target_os = "linux"))]
    let mut f = {
        let path =
            std::env::temp_dir().join(format!("{name}-{}-{}", std::process::id(), contents.len()));
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        let _ = std::fs::remove_file(&path);
        f
    };
    f.write_all(contents)?;
    f.write_all(&[0])?;
    f.flush()?;
    Ok(OwnedFd::from(f))
}

fn encode_png(raw: &[u8], w: u32, h: u32, format: wl_shm::Format) -> Result<Vec<u8>> {
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    let bgr = matches!(format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888);
    for px in raw.chunks_exact(4) {
        if bgr {
            rgb.extend_from_slice(&[px[2], px[1], px[0]]);
        } else {
            rgb.extend_from_slice(&[px[0], px[1], px[2]]);
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header()?;
        writer.write_image_data(&rgb)?;
    }
    Ok(out)
}

impl Keymap {
    /// The characters this keymap covers, for rebuilding a superset.
    pub fn text_chars(&self) -> String {
        self.chars_iter().collect()
    }
}

// ---- Dispatch ---------------------------------------------------------------

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        st: &mut Self,
        out: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = out.id().protocol_id();
        let Some(o) = st.pending_outputs.get_mut(&id) else {
            return;
        };
        match event {
            wl_output::Event::Mode {
                width,
                height,
                flags: WEnum::Value(f),
                ..
            } if f.contains(wl_output::Mode::Current) => {
                o.width = width;
                o.height = height;
            }
            wl_output::Event::Scale { factor } => o.scale = factor,
            wl_output::Event::Name { name } => o.name = name,
            wl_output::Event::Done => {
                let mut done = st.pending_outputs.remove(&id).unwrap();
                // Logical size for pointer extents.
                if done.scale > 1 {
                    done.width /= done.scale;
                    done.height /= done.scale;
                }
                st.outputs.retain(|x| x.name != done.name);
                st.outputs.push(done);
            }
            _ => {}
        }
    }
}

impl Dispatch<list::ExtForeignToplevelListV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &list::ExtForeignToplevelListV1,
        _: list::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
    wayland_client::event_created_child!(State, list::ExtForeignToplevelListV1, [
        list::EVT_TOPLEVEL_OPCODE => (handle::ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<handle::ExtForeignToplevelHandleV1, ()> for State {
    fn event(
        st: &mut Self,
        h: &handle::ExtForeignToplevelHandleV1,
        event: handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = h.id().protocol_id();
        match event {
            handle::Event::Identifier { identifier } => {
                st.pending.entry(id).or_default().identifier = identifier
            }
            handle::Event::AppId { app_id } => st.pending.entry(id).or_default().app_id = app_id,
            handle::Event::Title { title } => st.pending.entry(id).or_default().title = title,
            handle::Event::Done => {
                if let Some(t) = st.pending.remove(&id) {
                    if let Some(existing) = st.toplevels.iter_mut().find(|(x, _)| x == h) {
                        if !t.identifier.is_empty() {
                            existing.1.identifier = t.identifier;
                        }
                        if !t.app_id.is_empty() {
                            existing.1.app_id = t.app_id;
                        }
                        if !t.title.is_empty() {
                            existing.1.title = t.title;
                        }
                    } else {
                        st.toplevels.push((h.clone(), t));
                    }
                }
            }
            handle::Event::Closed => {
                st.toplevels.retain(|(x, _)| x != h);
                st.pending.remove(&id);
                h.destroy();
            }
            _ => {}
        }
    }
}

impl Dispatch<tseat::ExtTransientSeatV1, ()> for State {
    fn event(
        st: &mut Self,
        _: &tseat::ExtTransientSeatV1,
        event: tseat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            tseat::Event::Ready { global_name } => st.transient_ready = Some(global_name),
            tseat::Event::Denied => st.transient_ready = None,
            _ => {}
        }
    }
}

impl Dispatch<session::ExtImageCopyCaptureSessionV1, ()> for State {
    fn event(
        st: &mut Self,
        _: &session::ExtImageCopyCaptureSessionV1,
        event: session::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            session::Event::BufferSize { width, height } => {
                st.capture.width = width;
                st.capture.height = height;
            }
            // Prefer the first 32-bit format offered.
            session::Event::ShmFormat {
                format: WEnum::Value(f),
            } if st.capture.format.is_none()
                && matches!(
                    f,
                    wl_shm::Format::Xrgb8888
                        | wl_shm::Format::Argb8888
                        | wl_shm::Format::Xbgr8888
                        | wl_shm::Format::Abgr8888
                ) =>
            {
                st.capture.format = Some(f);
            }
            session::Event::Done => st.capture.session_done = true,
            session::Event::Stopped => st.capture.frame_failed = Some("session stopped".into()),
            _ => {}
        }
    }
}

impl Dispatch<frame::ExtImageCopyCaptureFrameV1, ()> for State {
    fn event(
        st: &mut Self,
        _: &frame::ExtImageCopyCaptureFrameV1,
        event: frame::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            frame::Event::Ready => st.capture.frame_ready = true,
            frame::Event::Failed { reason } => {
                st.capture.frame_failed = Some(format!("{reason:?}"))
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: tseat_mgr::ExtTransientSeatManagerV1);
delegate_noop!(State: vp_mgr::ZwlrVirtualPointerManagerV1);
delegate_noop!(State: vp::ZwlrVirtualPointerV1);
delegate_noop!(State: vk_mgr::ZwpVirtualKeyboardManagerV1);
delegate_noop!(State: vk::ZwpVirtualKeyboardV1);
delegate_noop!(State: capture_mgr::ExtImageCopyCaptureManagerV1);
delegate_noop!(State: out_source_mgr::ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(State: tl_source_mgr::ExtForeignToplevelImageCaptureSourceManagerV1);
delegate_noop!(State: source::ExtImageCaptureSourceV1);
