//! Capability probe: connect, list globals and toplevels, create a transient
//! seat with a virtual pointer and keyboard, capture an output to PNG.

use anyhow::{Context, Result};
use wayland_client::protocol::{wl_output, wl_registry, wl_seat, wl_shm};
use wayland_client::{delegate_noop, Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1, ext_foreign_toplevel_list_v1,
};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_foreign_toplevel_image_capture_source_manager_v1,
    ext_output_image_capture_source_manager_v1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1;
use wayland_protocols::ext::transient_seat::v1::client::{
    ext_transient_seat_manager_v1, ext_transient_seat_v1,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1;

#[derive(Default)]
struct Probe {
    globals: Vec<(String, u32)>,
    toplevels: Vec<(String, String, String)>, // identifier, app_id, title
    transient_seat_ready: Option<u32>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Probe {
    fn event(
        st: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            interface, version, ..
        } = event
        {
            st.globals.push((interface, version));
        }
    }
}

impl Dispatch<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, ()> for Probe {
    fn event(
        _: &mut Self,
        _: &ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1,
        _: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
    wayland_client::event_created_child!(Probe, ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1, ()> for Probe {
    fn event(
        st: &mut Self,
        _: &ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_foreign_toplevel_handle_v1::Event as E;
        match event {
            E::Identifier { identifier } => {
                st.toplevels
                    .push((identifier, String::new(), String::new()))
            }
            E::AppId { app_id } => {
                if let Some(t) = st.toplevels.last_mut() {
                    t.1 = app_id;
                }
            }
            E::Title { title } => {
                if let Some(t) = st.toplevels.last_mut() {
                    t.2 = title;
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ext_transient_seat_v1::ExtTransientSeatV1, ()> for Probe {
    fn event(
        st: &mut Self,
        _: &ext_transient_seat_v1::ExtTransientSeatV1,
        event: ext_transient_seat_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_transient_seat_v1::Event::Ready { global_name } = event {
            st.transient_seat_ready = Some(global_name);
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, wayland_client::globals::GlobalListContents> for Probe {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &wayland_client::globals::GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(Probe: ignore wl_seat::WlSeat);
delegate_noop!(Probe: ignore wl_output::WlOutput);
delegate_noop!(Probe: ignore wl_shm::WlShm);
delegate_noop!(Probe: ext_transient_seat_manager_v1::ExtTransientSeatManagerV1);
delegate_noop!(Probe: zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1);
delegate_noop!(Probe: zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1);
delegate_noop!(Probe: ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1);
delegate_noop!(Probe: ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(Probe: ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1);

pub fn run() -> Result<()> {
    let conn = Connection::connect_to_env()
        .context("connecting to the Wayland display (is WAYLAND_DISPLAY set?)")?;
    let display = conn.display();
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let registry = display.get_registry(&qh, ());
    let mut st = Probe::default();
    queue.roundtrip(&mut st)?;

    let need = [
        "ext_transient_seat_manager_v1",
        "zwlr_virtual_pointer_manager_v1",
        "zwp_virtual_keyboard_manager_v1",
        "ext_image_copy_capture_manager_v1",
        "ext_output_image_capture_source_manager_v1",
        "ext_foreign_toplevel_image_capture_source_manager_v1",
        "ext_foreign_toplevel_list_v1",
    ];
    println!("compositor globals: {}", st.globals.len());
    for n in need {
        let have = st.globals.iter().find(|(i, _)| i == n).map(|(_, v)| *v);
        println!(
            "  {:<58} {}",
            n,
            have.map(|v| format!("v{v}"))
                .unwrap_or_else(|| "MISSING".into())
        );
    }
    let _ = registry;
    // Bind the toplevel list to enumerate windows.
    let globals = wayland_client::globals::registry_queue_init::<Probe>(&conn).map(|(g, _)| g);
    if let Ok(globals) = globals {
        let mut q2 = conn.new_event_queue::<Probe>();
        let qh2 = q2.handle();
        let list: Result<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, _> =
            globals.bind(&qh2, 1..=1, ());
        if let Ok(_list) = list {
            let mut st2 = Probe::default();
            q2.roundtrip(&mut st2)?;
            q2.roundtrip(&mut st2)?;
            println!("toplevels: {}", st2.toplevels.len());
            for (id, app, title) in &st2.toplevels {
                println!("  {id} app_id={app:?} title={title:?}");
            }
            // Transient seat
            let tsm: Result<ext_transient_seat_manager_v1::ExtTransientSeatManagerV1, _> =
                globals.bind(&qh2, 1..=1, ());
            if let Ok(tsm) = tsm {
                let _seat = tsm.create(&qh2, ());
                q2.roundtrip(&mut st2)?;
                q2.roundtrip(&mut st2)?;
                match st2.transient_seat_ready {
                    Some(name) => println!("transient seat: ready, wl_seat global name {name}"),
                    None => println!("transient seat: not ready (denied?)"),
                }
            }
        }
    }
    Ok(())
}
