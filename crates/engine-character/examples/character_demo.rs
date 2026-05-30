//! Standalone character controller demo.
//!
//! Pushes [`CharacterCommand`]s based on keyboard input and drives the
//! controller each frame with physics collision against a ground plane.
//!
//! ```ignore
//! cargo run --example character_demo --features backend-vulkan
//! ```

#![allow(unsafe_code)]

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use engine_character::{CharacterCommand, CharacterController};
use engine_physics::{BodyType, Collider, ColliderShape, PhysicsWorld, RigidBody};
use engine_scene::components::Transform;
use engine_scene::World;
use glam::{Mat4, Vec3};
use platform::winit::window::Window;
use platform::{EventFlow, PlatformEvent, WindowApp, WindowDescriptor};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use render_core::{BufferDescriptor, BufferHandle, Device, MemoryHint};
use render_vulkan::device_impl::VulkanDevice;
use render_vulkan::shaders_embedded;

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

struct App {
    renderer: Option<Backend>,
    frames: u64,
    last_frame: Instant,
    keys: HashSet<u32>,
    ctrl: CharacterController,
    physics: Option<PhysicsWorld>,
    _world: World,
}

struct Backend {
    dev: VulkanDevice,
    vb: BufferHandle,
    ib: BufferHandle,
    ic: u32,
    w: f32,
    h: f32,
}

// ---------------------------------------------------------------------------
// Vertex / index buffers (cube + ground plane)
// ---------------------------------------------------------------------------

fn build_buffers(dev: &mut VulkanDevice) -> (BufferHandle, BufferHandle, u32) {
    let cv: &[f32] = &[
        -0.5,-0.5,0.5,0.0,0.0,1.0,0.0,0.0, 0.5,-0.5,0.5,0.0,0.0,1.0,1.0,0.0, 0.5,0.5,0.5,0.0,0.0,1.0,1.0,1.0, -0.5,0.5,0.5,0.0,0.0,1.0,0.0,1.0,
        -0.5,-0.5,-0.5,0.0,0.0,-1.0,0.0,0.0, 0.5,-0.5,-0.5,0.0,0.0,-1.0,1.0,0.0, 0.5,0.5,-0.5,0.0,0.0,-1.0,1.0,1.0, -0.5,0.5,-0.5,0.0,0.0,-1.0,0.0,1.0,
        0.5,-0.5,-0.5,1.0,0.0,0.0,0.0,0.0, 0.5,-0.5,0.5,1.0,0.0,0.0,1.0,0.0, 0.5,0.5,0.5,1.0,0.0,0.0,1.0,1.0, 0.5,0.5,-0.5,1.0,0.0,0.0,0.0,1.0,
        -0.5,-0.5,-0.5,-1.0,0.0,0.0,0.0,0.0, -0.5,-0.5,0.5,-1.0,0.0,0.0,1.0,0.0, -0.5,0.5,0.5,-1.0,0.0,0.0,1.0,1.0, -0.5,0.5,-0.5,-1.0,0.0,0.0,0.0,1.0,
        -0.5,0.5,-0.5,0.0,1.0,0.0,0.0,0.0, 0.5,0.5,-0.5,0.0,1.0,0.0,1.0,0.0, 0.5,0.5,0.5,0.0,1.0,0.0,1.0,1.0, -0.5,0.5,0.5,0.0,1.0,0.0,0.0,1.0,
        -0.5,-0.5,-0.5,0.0,-1.0,0.0,0.0,0.0, 0.5,-0.5,-0.5,0.0,-1.0,0.0,1.0,0.0, 0.5,-0.5,0.5,0.0,-1.0,0.0,1.0,1.0, -0.5,-0.5,0.5,0.0,-1.0,0.0,0.0,1.0,
    ];
    let pp = [[-10.0,-0.5,-10.0],[10.0,-0.5,-10.0],[10.0,-0.5,10.0],[-10.0,-0.5,10.0]];
    let pn = [0.0,1.0,0.0];
    let pu = [[0.0,0.0],[5.0,0.0],[5.0,5.0],[0.0,5.0]];
    let cvc = 24u32;

    let mut vert: Vec<u8> = vec![];
    for c in cv.chunks(8) { for &v in c { vert.extend_from_slice(&v.to_ne_bytes()); } }
    for i in 0..4 {
        let vals: [f32; 8] = [pp[i][0],pp[i][1],pp[i][2],pn[0],pn[1],pn[2],pu[i][0],pu[i][1]];
        for v in vals { vert.extend_from_slice(&v.to_ne_bytes()); }
    }
    let mut idx: Vec<u32> = (0..6u32).flat_map(|f| { let b = f*4; vec![b,b+1,b+2,b,b+2,b+3] }).collect();
    idx.extend_from_slice(&[cvc,cvc+1,cvc+2,cvc,cvc+2,cvc+3]);
    let ic = idx.len() as u32;
    let mut ib: Vec<u8> = vec![];
    for i in &idx { ib.extend_from_slice(&i.to_ne_bytes()); }

    let vb = dev.create_buffer(&BufferDescriptor{size_bytes:vert.len()as u64,usage_flags:render_core::BufferUsage(0),memory_hint:MemoryHint::CpuToGpu,debug_label:Some("v".into())}).unwrap();
    dev.write_buffer(vb, &vert, 0).unwrap();
    let ibh = dev.create_buffer(&BufferDescriptor{size_bytes:ib.len()as u64,usage_flags:render_core::BufferUsage(0),memory_hint:MemoryHint::CpuToGpu,debug_label:Some("i".into())}).unwrap();
    dev.write_buffer(ibh, &ib, 0).unwrap();
    (vb, ibh, ic)
}

// ---------------------------------------------------------------------------
// WindowApp impl
// ---------------------------------------------------------------------------

impl WindowApp for App {
    fn on_create(&mut self, window: Arc<Window>) {
        let s = window.inner_size();
        let dh = window.display_handle().unwrap().as_raw();
        let wh = window.window_handle().unwrap().as_raw();
        let val = std::env::var("ENGINE_VK_VALIDATION").is_ok();
        let mut dev = VulkanDevice::new(dh, wh, s.width.max(1), s.height.max(1), val,
            Some(std::path::Path::new("./pso_cache"))).unwrap();
        dev.set_mvp_shaders(shaders_embedded::FORWARD_VERT_SPV, shaders_embedded::FORWARD_FRAG_SPV);
        let (vb, ib, ic) = build_buffers(&mut dev);
        self.renderer = Some(Backend{dev, vb, ib, ic, w:s.width.max(1)as f32, h:s.height.max(1)as f32});
    }

    fn on_event(&mut self, _w: &Window, e: PlatformEvent) -> EventFlow {
        match e {
            // ── Input → commands ────────────────────────────────────────
            PlatformEvent::KeyPressed{key,..} => { self.keys.insert(key); EventFlow::Continue }
            PlatformEvent::KeyReleased{key,..} => { self.keys.remove(&key); EventFlow::Continue }
            PlatformEvent::Resized{..} => EventFlow::Continue,

            // ── Frame update ────────────────────────────────────────────
            PlatformEvent::Redraw => {
                // Frame timing
                let now = Instant::now();
                let el = now - self.last_frame;
                self.last_frame = now;
                let target = std::time::Duration::from_secs_f64(1.0/60.0);
                if el < target { std::thread::sleep(target - el); }
                let dt = el.as_secs_f32().min(0.05);

                // Build movement command from key state
                // (winit discriminant: W=41, A=19, S=37, D=22, Space=62)
                let mut dir = Vec3::ZERO;
                if self.keys.contains(&41) { dir.z -= 1.0; }
                if self.keys.contains(&37) { dir.z += 1.0; }
                if self.keys.contains(&19) { dir.x -= 1.0; }
                if self.keys.contains(&22) { dir.x += 1.0; }
                if dir.length_squared() > 0.0 { dir = dir.normalize(); }

                // Push command, then run one frame
                self.ctrl.push_command(CharacterCommand{direction: dir, desired_speed: 0.0, jump_requested: self.keys.contains(&62)});

                let input = engine_character::CharacterMovement {
                    direction: dir, wish_jump: false, delta_time: dt,
                };
                self.ctrl.update(&input, self.physics.as_ref());

                // ── Render ──────────────────────────────────────────────
                if let Some(ref mut r) = self.renderer {
                    let cp = self.ctrl.position();
                    let angle = self.frames as f32 * 0.02;
                    let eye = Vec3::new(5.0*angle.sin()+cp.x, 2.5+cp.y, 5.0*angle.cos()+cp.z);
                    let vp = Mat4::from_cols_array_2d(&[
                        [1.,0.,0.,0.],[0.,-1.,0.,0.],[0.,0.,0.5,0.],[0.,0.,0.5,1.]])
                        * Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, r.w/r.h, 0.1, 100.0)
                        * Mat4::look_at_rh(eye, cp, Vec3::Y);
                    let model = Mat4::from_translation(cp)
                        * Mat4::from_scale(Vec3::new(self.ctrl.radius*2.0, self.ctrl.height, self.ctrl.radius*2.0));
                    let mut ubo = Vec::with_capacity(176);
                    for v in model.to_cols_array_2d().iter().flatten() { ubo.extend_from_slice(&v.to_ne_bytes()); }
                    for v in vp.to_cols_array_2d().iter().flatten() { ubo.extend_from_slice(&v.to_ne_bytes()); }
                    let ld = Vec3::new(0.5,-0.707,0.5).normalize();
                    for v in &[ld.x,ld.y,ld.z,0.0] { ubo.extend_from_slice(&v.to_ne_bytes()); }
                    for v in &[1.5f32;4] { ubo.extend_from_slice(&v.to_ne_bytes()); }
                    for v in &[eye.x,eye.y,eye.z,1.0] { ubo.extend_from_slice(&v.to_ne_bytes()); }
                    r.dev.write_ubo_current(&ubo, 0);
                    if let Err(e) = r.dev.render_model_frame(r.vb, r.ib, r.ic) {
                        tracing::error!("{e}"); return EventFlow::Exit;
                    }
                }
                self.frames += 1;
                EventFlow::Continue
            }
            PlatformEvent::CloseRequested => EventFlow::Exit,
            _ => EventFlow::Continue,
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let mut w = World::new();
    let g = w.create_entity();
    w.add_component(g, RigidBody{body_type:BodyType::Static, ..RigidBody::default()});
    w.add_component(g, Collider{shape:ColliderShape::Cuboid{hx:10.0,hy:0.5,hz:10.0}, ..Collider::default()});
    w.add_component(g, Transform{translation:Vec3::new(0.0,-0.5,0.0), ..Transform::default()});
    let mut p = PhysicsWorld::new(Vec3::new(0.0,-9.81,0.0));
    p.sync_from_ecs(&w);
    let mut c = CharacterController::new();
    c.set_position(Vec3::new(0.0,3.0,0.0));
    let app = App{
        renderer:None, frames:0, last_frame:Instant::now(), keys:HashSet::new(),
        ctrl:c, physics:Some(p), _world:w,
    };
    platform::run(WindowDescriptor{title:"Character Demo".into(), width:1280, height:720}, app).unwrap();
}
