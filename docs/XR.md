# XR runtime

`engine-xr` separates portable XR frame/input contracts from a graphics API
bridge. `engine-core/subsystem-xr` owns `XrSystem`; ordinary desktop builds do
not load an XR runtime. Enabling `engine-xr/openxr-runtime` dynamically
discovers the OpenXR loader, creates an HMD instance/system, enables the Vulkan
binding and exposes its API requirements and runtime-recommended stereo
extents. `OpenXrRuntime::create_vulkan_session` validates an existing Vulkan
instance/device/queue, creates the native OpenXR session and returns a paired
runtime/compositor. Desktop builds without this feature never load OpenXR.

`XrFrameState` carries predicted display time, head/hand poses and two `XrView`
values (pose and asymmetric field of view). Its native
`stereo_camera_matrices` helper builds the two right-handed view/asymmetric
projection matrices. `XrActionSnapshot` exposes hand pose, select, trigger,
squeeze and thumbstick actions; the built-in binding set covers the Khronos
simple controller plus Oculus Touch and Valve Index profiles. Rendering hosts
call `GameLoop::begin_xr_frame`, render the returned left/right native targets,
and call `submit_xr_frame`. The engine enforces the frame order:

```text
poll events -> wait frame -> begin frame -> sync actions
            -> acquire/render stereo images -> submit -> end frame
```

The native pair owns the OpenXR session-state event loop, predicted view/head/
hand locations, ActionSet synchronisation, separate-eye Vulkan swapchains,
image wait/release and projection-layer `xrEndFrame` transaction. The runtime
and compositor remain injectable behind traits, so headless tests use a
deterministic fake. Calling `submit` without an active frame, re-acquiring an
unsubmitted image, submitting the wrong native target, or crossing predicted
frame identities is rejected instead of silently desynchronising OpenXR.

The versioned C# gameplay SDK publishes this state through
`EngineBehaviour.XR`. `XR.Frame` contains both asymmetric eye views plus
predicted head and left/right-hand poses, while `TryGetBoolean`,
`TryGetFloat`, `TryGetVector2`, and `TryGetPose` expose the native action
snapshot without asking project scripts to perform projection or tracking
math. The most recently predicted frame is retained after compositor
submission so the following simulation update can consume it safely.

`sandbox/target-desktop` includes the portable `subsystem-xr` bridge but does
not load an OpenXR loader merely because a desktop game starts. A Vulkan/OpenXR
render host opts into `engine-xr/openxr-runtime`, installs the paired native
runtime/compositor in `GameLoop::xr`, drives `begin_xr_frame`/stereo rendering/
`submit_xr_frame`, and the ordinary project-script context then becomes active.
