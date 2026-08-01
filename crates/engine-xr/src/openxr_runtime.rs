use std::{
    ffi::CString,
    sync::{Arc, Mutex},
};

use crate::{
    XrActionSnapshot, XrActionValue, XrCompositor, XrError, XrEye, XrFieldOfView, XrFrameState,
    XrPose, XrRuntime, XrSwapchainImage, XrView,
};

const VIEW_TYPE: openxr::ViewConfigurationType = openxr::ViewConfigurationType::PRIMARY_STEREO;

/// Raw Vulkan objects that have already been created with the extensions and
/// API version required by [`OpenXrRuntime::vulkan_requirements`].
#[derive(Clone, Copy, Debug)]
pub struct OpenXrVulkanBinding {
    pub instance: u64,
    pub physical_device: u64,
    pub device: u64,
    pub queue_family_index: u32,
    pub queue_index: u32,
    /// Vulkan `VkFormat` value used by both eye swapchains.
    pub color_format: i64,
    pub api_version: openxr::Version,
}

/// Dynamically loaded OpenXR instance and HMD system. The discovery object can
/// be queried before the renderer creates Vulkan, then consumed to create the
/// native session/compositor pair.
pub struct OpenXrRuntime {
    entry: openxr::Entry,
    instance: openxr::Instance,
    system: openxr::SystemId,
}

impl OpenXrRuntime {
    pub fn discover(application_name: &str, application_version: u32) -> Result<Self, XrError> {
        let application_name = CString::new(application_name)
            .map_err(|_| XrError::Runtime("application name contains a null byte".into()))?;
        // SAFETY: OpenXR's loader entry points are loaded through the crate's
        // platform loader and retained by `entry` for the instance lifetime.
        let entry = unsafe { openxr::Entry::load() }
            .map_err(|error| XrError::RuntimeUnavailable(error.to_string()))?;
        let extensions = entry
            .enumerate_extensions()
            .map_err(|error| XrError::Runtime(error.to_string()))?;
        if !extensions.khr_vulkan_enable && !extensions.khr_vulkan_enable2 {
            return Err(XrError::RuntimeUnavailable(
                "OpenXR runtime exposes neither KHR_vulkan_enable nor KHR_vulkan_enable2".into(),
            ));
        }
        let mut enabled = openxr::ExtensionSet::default();
        enabled.khr_vulkan_enable = extensions.khr_vulkan_enable;
        enabled.khr_vulkan_enable2 = extensions.khr_vulkan_enable2;
        let application_info = openxr::ApplicationInfo {
            application_name: application_name
                .to_str()
                .map_err(|error| XrError::Runtime(error.to_string()))?,
            application_version,
            engine_name: "engine",
            engine_version: 1,
            api_version: openxr::Version::new(1, 0, 0),
        };
        let instance = entry
            .create_instance(&application_info, &enabled, &[])
            .map_err(xr_runtime_error)?;
        let system = instance
            .system(openxr::FormFactor::HEAD_MOUNTED_DISPLAY)
            .map_err(|error| XrError::RuntimeUnavailable(error.to_string()))?;
        Ok(Self {
            entry,
            instance,
            system,
        })
    }

    pub fn entry(&self) -> &openxr::Entry {
        &self.entry
    }

    pub fn instance(&self) -> &openxr::Instance {
        &self.instance
    }

    pub fn system(&self) -> openxr::SystemId {
        self.system
    }

    pub fn vulkan_requirements(&self) -> Result<openxr::vulkan::Requirements, XrError> {
        self.instance
            .graphics_requirements::<openxr::Vulkan>(self.system)
            .map_err(xr_runtime_error)
    }

    pub fn recommended_stereo_extent(&self) -> Result<[[u32; 2]; 2], XrError> {
        let views = self
            .instance
            .enumerate_view_configuration_views(self.system, VIEW_TYPE)
            .map_err(xr_runtime_error)?;
        if views.len() != 2 {
            return Err(XrError::Runtime(format!(
                "PRIMARY_STEREO exposed {} views instead of two",
                views.len()
            )));
        }
        Ok([
            [
                views[0].recommended_image_rect_width,
                views[0].recommended_image_rect_height,
            ],
            [
                views[1].recommended_image_rect_width,
                views[1].recommended_image_rect_height,
            ],
        ])
    }

    /// Create a real OpenXR Vulkan session, tracked action set and two-eye
    /// compositor. The returned objects are paired through a shared frame
    /// transaction and must be installed into the same [`crate::XrSystem`].
    ///
    /// # Safety
    ///
    /// Every raw handle in `binding` must remain valid until both returned
    /// objects are dropped. The Vulkan instance/device must satisfy
    /// [`Self::vulkan_requirements`], the queue must belong to the device, and
    /// calls touching that queue must be externally synchronized by the host.
    pub unsafe fn create_vulkan_session(
        self,
        binding: OpenXrVulkanBinding,
    ) -> Result<(OpenXrVulkanSessionRuntime, OpenXrVulkanCompositor), XrError> {
        if binding.instance == 0 || binding.physical_device == 0 || binding.device == 0 {
            return Err(XrError::Graphics(
                "OpenXR Vulkan binding contains a null native handle".into(),
            ));
        }
        let requirements = self.vulkan_requirements()?;
        if binding.api_version < requirements.min_api_version_supported
            || binding.api_version > requirements.max_api_version_supported
        {
            return Err(XrError::Graphics(format!(
                "Vulkan API {} is outside OpenXR's supported range {}..={}",
                binding.api_version,
                requirements.min_api_version_supported,
                requirements.max_api_version_supported
            )));
        }
        let extents = self.recommended_stereo_extent()?;
        let blend_mode = preferred_blend_mode(&self.instance, self.system)?;
        let create_info = openxr::vulkan::SessionCreateInfo {
            instance: binding.instance as usize as *const std::ffi::c_void,
            physical_device: binding.physical_device as usize as *const std::ffi::c_void,
            device: binding.device as usize as *const std::ffi::c_void,
            queue_family_index: binding.queue_family_index,
            queue_index: binding.queue_index,
        };
        // SAFETY: upheld by the caller contract above and validated for null
        // handles/API version before passing the binding to OpenXR.
        let (session, frame_waiter, frame_stream) = unsafe {
            self.instance
                .create_session::<openxr::Vulkan>(self.system, &create_info)
        }
        .map_err(xr_runtime_error)?;
        if !session
            .enumerate_swapchain_formats()
            .map_err(xr_runtime_error)?
            .contains(&(binding.color_format as _))
        {
            return Err(XrError::Graphics(format!(
                "Vulkan format {} is not accepted by the OpenXR runtime",
                binding.color_format
            )));
        }

        let reference_type = preferred_reference_space(&session)?;
        let world_space = session
            .create_reference_space(reference_type, openxr::Posef::IDENTITY)
            .map_err(xr_runtime_error)?;
        let head_space = session
            .create_reference_space(openxr::ReferenceSpaceType::VIEW, openxr::Posef::IDENTITY)
            .map_err(xr_runtime_error)?;
        let compositor_space = session
            .create_reference_space(reference_type, openxr::Posef::IDENTITY)
            .map_err(xr_runtime_error)?;
        let actions = OpenXrActions::new(&self.instance, &session)?;
        let bridge = Arc::new(Mutex::new(OpenXrFrameBridge::new(frame_stream, blend_mode)));
        let swapchains = [
            EyeSwapchain::new(&session, binding.color_format, extents[0])?,
            EyeSwapchain::new(&session, binding.color_format, extents[1])?,
        ];
        let runtime = OpenXrVulkanSessionRuntime {
            _entry: self.entry,
            instance: self.instance,
            session: session.clone(),
            frame_waiter,
            bridge: Arc::clone(&bridge),
            world_space,
            head_space,
            actions,
            predicted_time: None,
            last_hands: [XrPose::default(); 2],
            running: false,
            exit: false,
        };
        let compositor = OpenXrVulkanCompositor {
            _session: session,
            bridge,
            world_space: compositor_space,
            swapchains,
        };
        Ok((runtime, compositor))
    }
}

pub struct OpenXrVulkanSessionRuntime {
    _entry: openxr::Entry,
    instance: openxr::Instance,
    session: openxr::Session<openxr::Vulkan>,
    frame_waiter: openxr::FrameWaiter,
    bridge: Arc<Mutex<OpenXrFrameBridge>>,
    world_space: openxr::Space,
    head_space: openxr::Space,
    actions: OpenXrActions,
    predicted_time: Option<openxr::Time>,
    last_hands: [XrPose; 2],
    running: bool,
    exit: bool,
}

impl XrRuntime for OpenXrVulkanSessionRuntime {
    fn poll_events(&mut self) -> Result<(), XrError> {
        // EventDataBuffer contains a transient raw `next` pointer and is not
        // Send; keeping it stack-local preserves the runtime's thread-transfer
        // contract without an unsafe Send implementation.
        let mut event_buffer = openxr::EventDataBuffer::new();
        while let Some(event) = self
            .instance
            .poll_event(&mut event_buffer)
            .map_err(xr_runtime_error)?
        {
            match event {
                openxr::Event::SessionStateChanged(event) => match event.state() {
                    openxr::SessionState::READY => {
                        self.session.begin(VIEW_TYPE).map_err(xr_runtime_error)?;
                        self.running = true;
                    }
                    openxr::SessionState::STOPPING => {
                        if self.running {
                            self.session.end().map_err(xr_runtime_error)?;
                        }
                        self.running = false;
                    }
                    openxr::SessionState::EXITING | openxr::SessionState::LOSS_PENDING => {
                        self.running = false;
                        self.exit = true;
                    }
                    _ => {}
                },
                openxr::Event::InstanceLossPending(_) => {
                    self.running = false;
                    self.exit = true;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn is_session_running(&self) -> bool {
        self.running
    }

    fn wait_frame(&mut self) -> Result<XrFrameState, XrError> {
        if !self.running {
            return Err(XrError::FrameLifecycle(
                "OpenXR wait requested while the session is not running",
            ));
        }
        let state = self.frame_waiter.wait().map_err(xr_runtime_error)?;
        self.predicted_time = Some(state.predicted_display_time);
        Ok(XrFrameState {
            predicted_display_time_nanoseconds: state.predicted_display_time.as_nanos(),
            should_render: state.should_render,
            ..XrFrameState::default()
        })
    }

    fn begin_frame(&mut self) -> Result<(), XrError> {
        let time = self.predicted_time.ok_or(XrError::FrameLifecycle(
            "OpenXR begin requested before wait_frame",
        ))?;
        lock_bridge(&self.bridge)?.begin(time)
    }

    fn sync_actions(&mut self) -> Result<XrActionSnapshot, XrError> {
        self.session
            .sync_actions(&[(&self.actions.set).into()])
            .map_err(xr_runtime_error)?;
        let time = self.predicted_time.ok_or(XrError::FrameLifecycle(
            "OpenXR actions requested before wait_frame",
        ))?;
        self.last_hands = self
            .actions
            .locate_hands(&self.session, &self.world_space, time)?;
        self.actions.snapshot(&self.session, self.last_hands)
    }

    fn locate_frame(&mut self, state: &mut XrFrameState) -> Result<(), XrError> {
        let time = self.predicted_time.ok_or(XrError::FrameLifecycle(
            "OpenXR locate requested before wait_frame",
        ))?;
        let (flags, views) = self
            .session
            .locate_views(VIEW_TYPE, time, &self.world_space)
            .map_err(xr_runtime_error)?;
        if views.len() != 2 {
            return Err(XrError::Runtime(format!(
                "OpenXR located {} PRIMARY_STEREO views instead of two",
                views.len()
            )));
        }
        state.views = [xr_view(views[0], flags), xr_view(views[1], flags)];
        state.head = xr_space_pose(
            self.head_space
                .locate(&self.world_space, time)
                .map_err(xr_runtime_error)?,
        );
        state.left_hand = self.last_hands[0];
        state.right_hand = self.last_hands[1];
        Ok(())
    }

    fn end_frame(&mut self, rendered: bool) -> Result<(), XrError> {
        let mut bridge = lock_bridge(&self.bridge)?;
        if rendered {
            bridge.finish_rendered()?;
        } else {
            bridge.finish_empty()?;
        }
        self.predicted_time = None;
        Ok(())
    }

    fn should_exit(&self) -> bool {
        self.exit
    }
}

pub struct OpenXrVulkanCompositor {
    // Keeps the native session alive after the runtime object is boxed.
    _session: openxr::Session<openxr::Vulkan>,
    bridge: Arc<Mutex<OpenXrFrameBridge>>,
    world_space: openxr::Space,
    swapchains: [EyeSwapchain; 2],
}

impl XrCompositor for OpenXrVulkanCompositor {
    fn begin_frame(&mut self, _state: &XrFrameState) -> Result<Vec<XrSwapchainImage>, XrError> {
        if self
            .swapchains
            .iter()
            .any(|swapchain| swapchain.acquired.is_some())
        {
            return Err(XrError::FrameLifecycle(
                "OpenXR swapchain image acquired twice without submission",
            ));
        }
        let left = self.swapchains[0].acquire(XrEye::Left)?;
        match self.swapchains[1].acquire(XrEye::Right) {
            Ok(right) => Ok(vec![left, right]),
            Err(error) => {
                let _ = self.swapchains[0].release();
                Err(error)
            }
        }
    }

    fn end_frame(
        &mut self,
        state: &XrFrameState,
        rendered_images: &[XrSwapchainImage],
    ) -> Result<(), XrError> {
        let validation = self.validate_images(rendered_images);
        let left_release = self.swapchains[0].release();
        let right_release = self.swapchains[1].release();
        validation?;
        left_release?;
        right_release?;
        lock_bridge(&self.bridge)?.finish_projection(&self.world_space, &self.swapchains, state)
    }
}

impl OpenXrVulkanCompositor {
    fn validate_images(&self, images: &[XrSwapchainImage]) -> Result<(), XrError> {
        if images.len() != 2 {
            return Err(XrError::Graphics(format!(
                "OpenXR compositor expected two rendered images, received {}",
                images.len()
            )));
        }
        for (index, eye) in [XrEye::Left, XrEye::Right].into_iter().enumerate() {
            let supplied = images
                .iter()
                .find(|image| image.eye == eye)
                .ok_or_else(|| XrError::Graphics(format!("missing {eye:?} eye image")))?;
            let expected = &self.swapchains[index];
            let acquired = expected.acquired.ok_or(XrError::FrameLifecycle(
                "OpenXR compositor submission has no acquired image",
            ))?;
            if supplied.image_index != acquired
                || supplied.native_image != expected.images[acquired as usize]
                || supplied.width != expected.width
                || supplied.height != expected.height
            {
                return Err(XrError::Graphics(format!(
                    "{eye:?} eye submission does not match the acquired OpenXR image"
                )));
            }
        }
        Ok(())
    }
}

struct EyeSwapchain {
    handle: openxr::Swapchain<openxr::Vulkan>,
    images: Vec<u64>,
    width: u32,
    height: u32,
    acquired: Option<u32>,
}

impl EyeSwapchain {
    fn new(
        session: &openxr::Session<openxr::Vulkan>,
        color_format: i64,
        extent: [u32; 2],
    ) -> Result<Self, XrError> {
        let handle = session
            .create_swapchain(&openxr::SwapchainCreateInfo {
                create_flags: openxr::SwapchainCreateFlags::EMPTY,
                usage_flags: openxr::SwapchainUsageFlags::COLOR_ATTACHMENT
                    | openxr::SwapchainUsageFlags::SAMPLED,
                format: color_format as _,
                sample_count: 1,
                width: extent[0],
                height: extent[1],
                face_count: 1,
                array_size: 1,
                mip_count: 1,
            })
            .map_err(xr_runtime_error)?;
        let images = handle
            .enumerate_images()
            .map_err(xr_runtime_error)?
            .into_iter()
            .collect::<Vec<_>>();
        if images.is_empty() {
            return Err(XrError::Graphics(
                "OpenXR returned an empty Vulkan swapchain".into(),
            ));
        }
        Ok(Self {
            handle,
            images,
            width: extent[0],
            height: extent[1],
            acquired: None,
        })
    }

    fn acquire(&mut self, eye: XrEye) -> Result<XrSwapchainImage, XrError> {
        let index = self.handle.acquire_image().map_err(xr_runtime_error)?;
        self.handle
            .wait_image(openxr::Duration::INFINITE)
            .map_err(xr_runtime_error)?;
        self.acquired = Some(index);
        let Some(native_image) = self.images.get(index as usize).copied() else {
            let _ = self.release();
            return Err(XrError::Graphics(
                "OpenXR returned an out-of-range swapchain image index".into(),
            ));
        };
        Ok(XrSwapchainImage {
            eye,
            image_index: index,
            array_layer: 0,
            native_image,
            width: self.width,
            height: self.height,
        })
    }

    fn release(&mut self) -> Result<(), XrError> {
        if self.acquired.take().is_none() {
            return Ok(());
        }
        self.handle.release_image().map_err(xr_runtime_error)
    }
}

struct OpenXrFrameBridge {
    stream: openxr::FrameStream<openxr::Vulkan>,
    blend_mode: openxr::EnvironmentBlendMode,
    predicted_time: Option<openxr::Time>,
    projection_submitted: bool,
}

impl OpenXrFrameBridge {
    fn new(
        stream: openxr::FrameStream<openxr::Vulkan>,
        blend_mode: openxr::EnvironmentBlendMode,
    ) -> Self {
        Self {
            stream,
            blend_mode,
            predicted_time: None,
            projection_submitted: false,
        }
    }

    fn begin(&mut self, predicted_time: openxr::Time) -> Result<(), XrError> {
        if self.predicted_time.is_some() {
            return Err(XrError::FrameLifecycle(
                "OpenXR frame begun twice without end",
            ));
        }
        self.stream.begin().map_err(xr_runtime_error)?;
        self.predicted_time = Some(predicted_time);
        self.projection_submitted = false;
        Ok(())
    }

    fn finish_projection(
        &mut self,
        space: &openxr::Space,
        swapchains: &[EyeSwapchain; 2],
        state: &XrFrameState,
    ) -> Result<(), XrError> {
        let predicted_time = self.predicted_time.ok_or(XrError::FrameLifecycle(
            "OpenXR projection submitted without begin_frame",
        ))?;
        if predicted_time.as_nanos() != state.predicted_display_time_nanoseconds {
            return Err(XrError::FrameLifecycle(
                "OpenXR projection belongs to a different predicted frame",
            ));
        }
        let projection_views = [
            projection_view(&swapchains[0], state.views[0]),
            projection_view(&swapchains[1], state.views[1]),
        ];
        let layer = openxr::CompositionLayerProjection::new()
            .space(space)
            .views(&projection_views);
        self.stream
            .end(predicted_time, self.blend_mode, &[&layer])
            .map_err(xr_runtime_error)?;
        self.projection_submitted = true;
        Ok(())
    }

    fn finish_rendered(&mut self) -> Result<(), XrError> {
        if !self.projection_submitted {
            return Err(XrError::FrameLifecycle(
                "OpenXR rendered frame ended without a projection layer",
            ));
        }
        self.predicted_time = None;
        self.projection_submitted = false;
        Ok(())
    }

    fn finish_empty(&mut self) -> Result<(), XrError> {
        let Some(predicted_time) = self.predicted_time.take() else {
            return Err(XrError::FrameLifecycle(
                "OpenXR frame ended without begin_frame",
            ));
        };
        if !self.projection_submitted {
            self.stream
                .end(predicted_time, self.blend_mode, &[])
                .map_err(xr_runtime_error)?;
        }
        self.projection_submitted = false;
        Ok(())
    }
}

struct OpenXrActions {
    set: openxr::ActionSet,
    hand_pose: openxr::Action<openxr::Posef>,
    select: openxr::Action<bool>,
    trigger: openxr::Action<f32>,
    squeeze: openxr::Action<f32>,
    thumbstick: openxr::Action<openxr::Vector2f>,
    hand_paths: [openxr::Path; 2],
    hand_spaces: [openxr::Space; 2],
}

impl OpenXrActions {
    fn new(
        instance: &openxr::Instance,
        session: &openxr::Session<openxr::Vulkan>,
    ) -> Result<Self, XrError> {
        let hand_paths = [
            instance
                .string_to_path("/user/hand/left")
                .map_err(xr_runtime_error)?,
            instance
                .string_to_path("/user/hand/right")
                .map_err(xr_runtime_error)?,
        ];
        let set = instance
            .create_action_set("gameplay", "Gameplay", 0)
            .map_err(xr_runtime_error)?;
        let hand_pose = set
            .create_action::<openxr::Posef>("hand_pose", "Hand Pose", &hand_paths)
            .map_err(xr_runtime_error)?;
        let select = set
            .create_action::<bool>("select", "Select", &hand_paths)
            .map_err(xr_runtime_error)?;
        let trigger = set
            .create_action::<f32>("trigger", "Trigger", &hand_paths)
            .map_err(xr_runtime_error)?;
        let squeeze = set
            .create_action::<f32>("squeeze", "Squeeze", &hand_paths)
            .map_err(xr_runtime_error)?;
        let thumbstick = set
            .create_action::<openxr::Vector2f>("thumbstick", "Thumbstick", &hand_paths)
            .map_err(xr_runtime_error)?;
        let simple_profile = instance
            .string_to_path("/interaction_profiles/khr/simple_controller")
            .map_err(xr_runtime_error)?;
        let bindings = [
            openxr::Binding::new(
                &hand_pose,
                instance
                    .string_to_path("/user/hand/left/input/grip/pose")
                    .map_err(xr_runtime_error)?,
            ),
            openxr::Binding::new(
                &hand_pose,
                instance
                    .string_to_path("/user/hand/right/input/grip/pose")
                    .map_err(xr_runtime_error)?,
            ),
            openxr::Binding::new(
                &select,
                instance
                    .string_to_path("/user/hand/left/input/select/click")
                    .map_err(xr_runtime_error)?,
            ),
            openxr::Binding::new(
                &select,
                instance
                    .string_to_path("/user/hand/right/input/select/click")
                    .map_err(xr_runtime_error)?,
            ),
        ];
        instance
            .suggest_interaction_profile_bindings(simple_profile, &bindings)
            .map_err(xr_runtime_error)?;
        suggest_analog_controller_profile(
            instance,
            "/interaction_profiles/oculus/touch_controller",
            &hand_pose,
            &trigger,
            &squeeze,
            &thumbstick,
        )?;
        suggest_analog_controller_profile(
            instance,
            "/interaction_profiles/valve/index_controller",
            &hand_pose,
            &trigger,
            &squeeze,
            &thumbstick,
        )?;
        session
            .attach_action_sets(&[&set])
            .map_err(xr_runtime_error)?;
        let hand_spaces = [
            hand_pose
                .create_space(session, hand_paths[0], openxr::Posef::IDENTITY)
                .map_err(xr_runtime_error)?,
            hand_pose
                .create_space(session, hand_paths[1], openxr::Posef::IDENTITY)
                .map_err(xr_runtime_error)?,
        ];
        Ok(Self {
            set,
            hand_pose,
            select,
            trigger,
            squeeze,
            thumbstick,
            hand_paths,
            hand_spaces,
        })
    }

    fn locate_hands(
        &self,
        session: &openxr::Session<openxr::Vulkan>,
        world_space: &openxr::Space,
        time: openxr::Time,
    ) -> Result<[XrPose; 2], XrError> {
        let mut hands = [XrPose::default(); 2];
        for (index, hand) in hands.iter_mut().enumerate() {
            if self
                .hand_pose
                .is_active(session, self.hand_paths[index])
                .unwrap_or(false)
            {
                *hand = xr_space_pose(
                    self.hand_spaces[index]
                        .locate(world_space, time)
                        .map_err(xr_runtime_error)?,
                );
            }
        }
        Ok(hands)
    }

    fn snapshot(
        &self,
        session: &openxr::Session<openxr::Vulkan>,
        hands: [XrPose; 2],
    ) -> Result<XrActionSnapshot, XrError> {
        let mut snapshot = XrActionSnapshot::default();
        for (index, suffix) in ["left", "right"].into_iter().enumerate() {
            let select = self
                .select
                .state(session, self.hand_paths[index])
                .map_err(xr_runtime_error)?;
            let squeeze = self
                .squeeze
                .state(session, self.hand_paths[index])
                .map_err(xr_runtime_error)?;
            let trigger = self
                .trigger
                .state(session, self.hand_paths[index])
                .map_err(xr_runtime_error)?;
            let thumbstick = self
                .thumbstick
                .state(session, self.hand_paths[index])
                .map_err(xr_runtime_error)?;
            snapshot.values.insert(
                format!("select.{suffix}"),
                XrActionValue::Boolean(
                    (select.is_active && select.current_state)
                        || (trigger.is_active && trigger.current_state >= 0.5),
                ),
            );
            snapshot.values.insert(
                format!("trigger.{suffix}"),
                XrActionValue::Float(if trigger.is_active {
                    trigger.current_state
                } else {
                    0.0
                }),
            );
            snapshot.values.insert(
                format!("squeeze.{suffix}"),
                XrActionValue::Float(if squeeze.is_active {
                    squeeze.current_state
                } else {
                    0.0
                }),
            );
            snapshot.values.insert(
                format!("thumbstick.{suffix}"),
                XrActionValue::Vector2(if thumbstick.is_active {
                    [thumbstick.current_state.x, thumbstick.current_state.y]
                } else {
                    [0.0; 2]
                }),
            );
            snapshot.values.insert(
                format!("hand_pose.{suffix}"),
                XrActionValue::Pose(hands[index]),
            );
        }
        Ok(snapshot)
    }
}

fn suggest_analog_controller_profile(
    instance: &openxr::Instance,
    profile: &str,
    hand_pose: &openxr::Action<openxr::Posef>,
    trigger: &openxr::Action<f32>,
    squeeze: &openxr::Action<f32>,
    thumbstick: &openxr::Action<openxr::Vector2f>,
) -> Result<(), XrError> {
    let profile = instance.string_to_path(profile).map_err(xr_runtime_error)?;
    let path = |value| instance.string_to_path(value).map_err(xr_runtime_error);
    let bindings = [
        openxr::Binding::new(hand_pose, path("/user/hand/left/input/grip/pose")?),
        openxr::Binding::new(hand_pose, path("/user/hand/right/input/grip/pose")?),
        openxr::Binding::new(trigger, path("/user/hand/left/input/trigger/value")?),
        openxr::Binding::new(trigger, path("/user/hand/right/input/trigger/value")?),
        openxr::Binding::new(squeeze, path("/user/hand/left/input/squeeze/value")?),
        openxr::Binding::new(squeeze, path("/user/hand/right/input/squeeze/value")?),
        openxr::Binding::new(thumbstick, path("/user/hand/left/input/thumbstick")?),
        openxr::Binding::new(thumbstick, path("/user/hand/right/input/thumbstick")?),
    ];
    match instance.suggest_interaction_profile_bindings(profile, &bindings) {
        Ok(()) | Err(openxr::sys::Result::ERROR_PATH_UNSUPPORTED) => Ok(()),
        Err(error) => Err(xr_runtime_error(error)),
    }
}

fn preferred_reference_space(
    session: &openxr::Session<openxr::Vulkan>,
) -> Result<openxr::ReferenceSpaceType, XrError> {
    let supported = session
        .enumerate_reference_spaces()
        .map_err(xr_runtime_error)?;
    if supported.contains(&openxr::ReferenceSpaceType::STAGE) {
        Ok(openxr::ReferenceSpaceType::STAGE)
    } else if supported.contains(&openxr::ReferenceSpaceType::LOCAL) {
        Ok(openxr::ReferenceSpaceType::LOCAL)
    } else {
        Err(XrError::RuntimeUnavailable(
            "OpenXR runtime exposes neither STAGE nor LOCAL reference space".into(),
        ))
    }
}

fn preferred_blend_mode(
    instance: &openxr::Instance,
    system: openxr::SystemId,
) -> Result<openxr::EnvironmentBlendMode, XrError> {
    let modes = instance
        .enumerate_environment_blend_modes(system, VIEW_TYPE)
        .map_err(xr_runtime_error)?;
    modes
        .iter()
        .copied()
        .find(|mode| *mode == openxr::EnvironmentBlendMode::OPAQUE)
        .or_else(|| modes.first().copied())
        .ok_or_else(|| {
            XrError::RuntimeUnavailable("OpenXR exposes no environment blend mode".into())
        })
}

fn projection_view(
    swapchain: &EyeSwapchain,
    view: XrView,
) -> openxr::CompositionLayerProjectionView<'_, openxr::Vulkan> {
    openxr::CompositionLayerProjectionView::new()
        .pose(portable_pose(view.pose))
        .fov(openxr::Fovf {
            angle_left: view.fov.angle_left,
            angle_right: view.fov.angle_right,
            angle_up: view.fov.angle_up,
            angle_down: view.fov.angle_down,
        })
        .sub_image(
            openxr::SwapchainSubImage::new()
                .swapchain(&swapchain.handle)
                .image_array_index(0)
                .image_rect(openxr::Rect2Di {
                    offset: openxr::Offset2Di { x: 0, y: 0 },
                    extent: openxr::Extent2Di {
                        width: swapchain.width as i32,
                        height: swapchain.height as i32,
                    },
                }),
        )
}

fn xr_view(view: openxr::View, flags: openxr::ViewStateFlags) -> XrView {
    XrView {
        pose: XrPose {
            orientation: [
                view.pose.orientation.x,
                view.pose.orientation.y,
                view.pose.orientation.z,
                view.pose.orientation.w,
            ],
            position: [
                view.pose.position.x,
                view.pose.position.y,
                view.pose.position.z,
            ],
            orientation_valid: flags.contains(openxr::ViewStateFlags::ORIENTATION_VALID),
            position_valid: flags.contains(openxr::ViewStateFlags::POSITION_VALID),
            tracked: flags.intersects(
                openxr::ViewStateFlags::ORIENTATION_TRACKED
                    | openxr::ViewStateFlags::POSITION_TRACKED,
            ),
        },
        fov: XrFieldOfView {
            angle_left: view.fov.angle_left,
            angle_right: view.fov.angle_right,
            angle_up: view.fov.angle_up,
            angle_down: view.fov.angle_down,
        },
    }
}

fn xr_space_pose(location: openxr::SpaceLocation) -> XrPose {
    let flags = location.location_flags;
    XrPose {
        orientation: [
            location.pose.orientation.x,
            location.pose.orientation.y,
            location.pose.orientation.z,
            location.pose.orientation.w,
        ],
        position: [
            location.pose.position.x,
            location.pose.position.y,
            location.pose.position.z,
        ],
        orientation_valid: flags.contains(openxr::SpaceLocationFlags::ORIENTATION_VALID),
        position_valid: flags.contains(openxr::SpaceLocationFlags::POSITION_VALID),
        tracked: flags.intersects(
            openxr::SpaceLocationFlags::ORIENTATION_TRACKED
                | openxr::SpaceLocationFlags::POSITION_TRACKED,
        ),
    }
}

fn portable_pose(pose: XrPose) -> openxr::Posef {
    openxr::Posef {
        orientation: openxr::Quaternionf {
            x: pose.orientation[0],
            y: pose.orientation[1],
            z: pose.orientation[2],
            w: pose.orientation[3],
        },
        position: openxr::Vector3f {
            x: pose.position[0],
            y: pose.position[1],
            z: pose.position[2],
        },
    }
}

fn lock_bridge(
    bridge: &Arc<Mutex<OpenXrFrameBridge>>,
) -> Result<std::sync::MutexGuard<'_, OpenXrFrameBridge>, XrError> {
    bridge
        .lock()
        .map_err(|_| XrError::Runtime("OpenXR frame bridge mutex is poisoned".into()))
}

fn xr_runtime_error(error: openxr::sys::Result) -> XrError {
    XrError::Runtime(error.to_string())
}
