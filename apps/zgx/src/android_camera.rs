//! Reading a computer's connect code with the phone's camera (Android):
//! the NDK's camera (camera2ndk) into an image reader (mediandk), each
//! frame's brightness (the Y plane of YUV 4:2:0) looked through for a QR
//! code by rqrr, on a thread of its own. No Java of ours: the camera
//! permission is asked of the activity through JNI, and its answer read
//! back by asking again (a NativeActivity isn't told it).
//!
//! The host's side of src/phone_link.rs's `phone.scan`, `phone.scan_stop`
//! and `phone.connect`. What the camera sees is never kept or logged: only
//! how many frames it looked at, and, for the code it found, that it is one.

use std::ffi::{c_int, c_void, CStr};
use std::time::{Duration, Instant};

use zgx_desktop::android::AndroidApp;
use zgx_script::{ScriptHost, Value};

/// The live scan: a newer one or `phone.scan_stop` ends the one before.
static SCANS: crate::phone_link::Attempts = crate::phone_link::Attempts::new();
/// The camera, held by one scan's thread at a time: one that was ended
/// lets go of it (as its thread ends, a panic included) before the next
/// opens it.
static CAMERA: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// The camera went (disconnected, or an error) under the scan holding it.
static LOST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// How long a scan looks before giving up, and how long it waits for the
/// person to answer the camera permission.
const SCAN_FOR: Duration = Duration::from_secs(120);
const ASK_FOR: Duration = Duration::from_secs(60);
/// The frames asked of the camera (a size every camera gives), and how
/// often one is looked at.
const WIDTH: i32 = 640;
const HEIGHT: i32 = 480;
const EVERY: Duration = Duration::from_millis(150);

/// What the scanning thread tells the UI.
enum Progress {
    /// "asking" (the permission) or "scanning", with the frames looked at.
    State(&'static str, u32),
    /// The code found, or why there is none.
    Done(Result<String, &'static str>),
}

fn status_value(state: &str, frames: u32) -> Value {
    Value::record([("state", Value::from(state)), ("frames", Value::from(frames as f64))])
}

pub fn install(host: &ScriptHost) {
    let status = host.shared("phone.scan", status_value("idle", 0));
    host.async_action("phone.scan", move |_| {
        let status = status.clone();
        async move {
            crate::plugins::preview::refuse("phone.scan")?;
            let attempt = SCANS.begin();
            let Some(app) = zgx_desktop::android::app() else {
                return Err("camera".to_owned());
            };
            let (tx, mut rx) = zgx::task::latest::<Progress>();
            let spawned = std::thread::Builder::new().name("zend-camera".into()).spawn(move || {
                let found = scan(&app, attempt, &|state, frames| {
                    tx.send(Progress::State(state, frames));
                });
                tx.send(Progress::Done(found));
            });
            if spawned.is_err() {
                return Err("camera".to_owned());
            }
            loop {
                let progress = rx.recv().await;
                // An ended scan says nothing more: a newer one has the page.
                if !SCANS.live(attempt) {
                    return Err("stopped".to_owned());
                }
                match progress {
                    Some(Progress::State(state, frames)) => {
                        status.set(status_value(state, frames));
                    }
                    Some(Progress::Done(found)) => {
                        status.set(status_value("idle", 0));
                        return found.map(|code| Value::from(code.as_str())).map_err(str::to_owned);
                    }
                    None => {
                        status.set(status_value("idle", 0));
                        return Err("camera".to_owned());
                    }
                }
            }
        }
    });
    host.action("phone.scan_stop", |_| SCANS.stop());
    host.async_action("phone.connect", |arg| async move {
        crate::plugins::preview::refuse("phone.connect")?;
        let code = arg.field("code").map(crate::auth::text).unwrap_or_default();
        crate::phone_link::connect(&code).await
    });
}

/// How long after asking for the permission the system's dialog must have
/// taken the window's focus: a permission refused for good shows none.
const DIALOG_WITHIN: Duration = Duration::from_millis(1500);

/// Ask for the camera, then look until a connect code shows, the scan is
/// ended (`attempt` no longer live), or [`SCAN_FOR`] passes.
fn scan(app: &AndroidApp, attempt: u64, tell: &dyn Fn(&'static str, u32)) -> Result<String, &'static str> {
    let live = || SCANS.live(attempt);
    match camera_allowed(app, false) {
        Ok(true) => {}
        Ok(false) => {
            tell("asking", 0);
            log::info!("zend: camera: asking for the permission");
            camera_allowed(app, true).map_err(|_| "camera")?;
            // The answer comes as the system's dialog closes (the window has
            // its focus back): refused then, it's "denied" at once.
            let asked = Instant::now();
            let mut dialog = false;
            loop {
                if !live() {
                    return Err("stopped");
                }
                std::thread::sleep(Duration::from_millis(100));
                if camera_allowed(app, false).unwrap_or(false) {
                    break;
                }
                let focused = window_focused(app).unwrap_or(true);
                dialog |= !focused;
                let closed = if dialog { focused } else { asked.elapsed() > DIALOG_WITHIN };
                if closed || asked.elapsed() > ASK_FOR {
                    log::info!("zend: camera: the permission wasn't given");
                    return Err("denied");
                }
            }
        }
        Err(error) => {
            log::error!("zend: camera: the permission couldn't be read: {error}");
            return Err("camera");
        }
    }
    // The camera once an ended scan has let go of it.
    let _held = CAMERA.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !live() {
        return Err("stopped");
    }
    LOST.store(false, std::sync::atomic::Ordering::SeqCst);
    tell("scanning", 0);
    // SAFETY: Camera owns what it opens and frees it as it drops, on this
    // thread; nothing of it leaves.
    let camera = unsafe { Camera::open() }?;
    log::info!("zend: camera: scanning");
    let started = Instant::now();
    let mut frames = 0u32;
    let mut grey = Vec::new();
    loop {
        if !live() {
            log::info!("zend: camera: stopped after {frames} frames");
            return Err("stopped");
        }
        if LOST.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("camera");
        }
        if started.elapsed() > SCAN_FOR {
            log::info!("zend: camera: no code in {frames} frames");
            return Err("timeout");
        }
        std::thread::sleep(EVERY);
        // SAFETY: as above.
        let Some((w, h)) = (unsafe { camera.latest(&mut grey) }) else { continue };
        frames += 1;
        if frames % 10 == 1 {
            tell("scanning", frames);
        }
        if let Some(text) = read_code(w, h, &grey) {
            log::info!("zend: camera: a connect code, in frame {frames} ({w}x{h})");
            return Ok(text);
        }
    }
}

/// The text of a connect code in a greyscale frame `w` by `h` (other QR
/// codes are passed over).
pub fn read_code(w: usize, h: usize, grey: &[u8]) -> Option<String> {
    let mut image = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| grey[y * w + x]);
    image
        .detect_grids()
        .into_iter()
        .filter_map(|grid| grid.decode().ok().map(|(_, text)| text))
        .find(|text| crate::phone_link::decode(text).is_ok())
}

/// Whether this app may use the camera (`checkSelfPermission`); with
/// `ask`, ask the person for it (`requestPermissions`), which returns at
/// once: they answer in the system's dialog.
fn camera_allowed(app: &AndroidApp, ask: bool) -> jni::errors::Result<bool> {
    use jni::objects::{JObject, JValue};
    // SAFETY: the activity's VM and activity, alive while the app runs.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let asked = |env: &mut jni::JNIEnv| -> jni::errors::Result<bool> {
        let name = env.new_string("android.permission.CAMERA")?;
        let granted = env.call_method(&activity, "checkSelfPermission", "(Ljava/lang/String;)I", &[JValue::Object(&name)])?.i()? == 0;
        if !granted && ask {
            let names = env.new_object_array(1, "java/lang/String", &name)?;
            env.call_method(&activity, "requestPermissions", "([Ljava/lang/String;I)V", &[JValue::Object(&names), JValue::Int(7501)])?;
        }
        Ok(granted)
    };
    let answer = asked(&mut env);
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    answer
}

/// Whether the activity's window has the focus (`hasWindowFocus`): not
/// while the system's permission dialog is over it.
fn window_focused(app: &AndroidApp) -> jni::errors::Result<bool> {
    use jni::objects::JObject;
    // SAFETY: as in camera_allowed.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let focused = env.call_method(&activity, "hasWindowFocus", "()Z", &[]).and_then(|v| v.z());
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    focused
}

/// The back camera (else the first), repeating frames into an image reader.
struct Camera {
    manager: *mut ffi::ACameraManager,
    device: *mut ffi::ACameraDevice,
    reader: *mut ffi::AImageReader,
    container: *mut ffi::ACaptureSessionOutputContainer,
    output: *mut ffi::ACaptureSessionOutput,
    target: *mut ffi::ACameraOutputTarget,
    request: *mut ffi::ACaptureRequest,
    session: *mut ffi::ACameraCaptureSession,
    // (Kept for the device's and the session's life.)
    device_callbacks: Box<ffi::ACameraDevice_StateCallbacks>,
    session_callbacks: Box<ffi::ACameraCaptureSession_stateCallbacks>,
}

extern "C" fn on_device_disconnected(_: *mut c_void, _: *mut ffi::ACameraDevice) {
    log::warn!("zend: camera: disconnected");
    LOST.store(true, std::sync::atomic::Ordering::SeqCst);
}
extern "C" fn on_device_error(_: *mut c_void, _: *mut ffi::ACameraDevice, error: c_int) {
    log::warn!("zend: camera: error {error}");
    LOST.store(true, std::sync::atomic::Ordering::SeqCst);
}
extern "C" fn on_session(_: *mut c_void, _: *mut ffi::ACameraCaptureSession) {}

fn failed(what: &str, status: c_int) -> &'static str {
    log::error!("zend: camera: {what}: {status}");
    "camera"
}

// (Each of these is all FFI calls, one after the other: unsafe throughout.)
#[allow(unsafe_op_in_unsafe_fn)]
impl Camera {
    /// # Safety
    /// On one thread, which drops it.
    unsafe fn open() -> Result<Camera, &'static str> {
        use ffi::*;
        let mut camera = Camera {
            manager: ACameraManager_create(),
            device: std::ptr::null_mut(),
            reader: std::ptr::null_mut(),
            container: std::ptr::null_mut(),
            output: std::ptr::null_mut(),
            target: std::ptr::null_mut(),
            request: std::ptr::null_mut(),
            session: std::ptr::null_mut(),
            device_callbacks: Box::new(ACameraDevice_StateCallbacks {
                context: std::ptr::null_mut(),
                on_disconnected: on_device_disconnected,
                on_error: on_device_error,
            }),
            session_callbacks: Box::new(ACameraCaptureSession_stateCallbacks {
                context: std::ptr::null_mut(),
                on_closed: on_session,
                on_ready: on_session,
                on_active: on_session,
            }),
        };
        if camera.manager.is_null() {
            return Err("camera");
        }
        let id = camera.pick().ok_or("no_camera")?;
        let callbacks: *mut ACameraDevice_StateCallbacks = &mut *camera.device_callbacks;
        let status = ACameraManager_openCamera(camera.manager, id.as_ptr(), callbacks, &mut camera.device);
        if status != 0 {
            return Err(failed("open", status));
        }
        let status = AImageReader_new(WIDTH, HEIGHT, AIMAGE_FORMAT_YUV_420_888, 2, &mut camera.reader);
        if status != 0 {
            return Err(failed("image reader", status));
        }
        let mut window = std::ptr::null_mut();
        let status = AImageReader_getWindow(camera.reader, &mut window);
        if status != 0 {
            return Err(failed("reader's window", status));
        }
        let status = ACaptureSessionOutputContainer_create(&mut camera.container);
        if status != 0 {
            return Err(failed("outputs", status));
        }
        let status = ACaptureSessionOutput_create(window, &mut camera.output);
        if status != 0 {
            return Err(failed("output", status));
        }
        let status = ACaptureSessionOutputContainer_add(camera.container, camera.output);
        if status != 0 {
            return Err(failed("add output", status));
        }
        let status = ACameraOutputTarget_create(window, &mut camera.target);
        if status != 0 {
            return Err(failed("target", status));
        }
        let status = ACameraDevice_createCaptureRequest(camera.device, TEMPLATE_PREVIEW, &mut camera.request);
        if status != 0 {
            return Err(failed("request", status));
        }
        let status = ACaptureRequest_addTarget(camera.request, camera.target);
        if status != 0 {
            return Err(failed("add target", status));
        }
        let status = ACameraDevice_createCaptureSession(camera.device, camera.container, &*camera.session_callbacks, &mut camera.session);
        if status != 0 {
            return Err(failed("session", status));
        }
        let mut requests = [camera.request];
        let status = ACameraCaptureSession_setRepeatingRequest(camera.session, std::ptr::null_mut(), 1, requests.as_mut_ptr(), std::ptr::null_mut());
        if status != 0 {
            return Err(failed("repeating", status));
        }
        Ok(camera)
    }

    /// The id of the back camera, else of the first one there is.
    unsafe fn pick(&self) -> Option<std::ffi::CString> {
        use ffi::*;
        let mut list = std::ptr::null_mut();
        if ACameraManager_getCameraIdList(self.manager, &mut list) != 0 || list.is_null() {
            return None;
        }
        let ids: Vec<std::ffi::CString> = (0..(*list).num_cameras.max(0) as usize)
            .map(|i| CStr::from_ptr(*(*list).camera_ids.add(i)).to_owned())
            .collect();
        ACameraManager_deleteCameraIdList(list);
        let back = ids.iter().find(|id| {
            let mut meta = std::ptr::null_mut();
            if ACameraManager_getCameraCharacteristics(self.manager, id.as_ptr(), &mut meta) != 0 {
                return false;
            }
            let mut entry = ACameraMetadata_const_entry { tag: 0, kind: 0, count: 0, data: std::ptr::null() };
            let facing = ACameraMetadata_getConstEntry(meta, ACAMERA_LENS_FACING, &mut entry) == 0
                && entry.count > 0
                && !entry.data.is_null()
                && *(entry.data as *const u8) == ACAMERA_LENS_FACING_BACK;
            ACameraMetadata_free(meta);
            facing
        });
        log::info!("zend: camera: {} cameras, {}", ids.len(), if back.is_some() { "a back one" } else { "no back one" });
        back.or(ids.first()).cloned()
    }

    /// The newest frame's brightness into `grey` (row after row) and its
    /// size, if one came since the last.
    unsafe fn latest(&self, grey: &mut Vec<u8>) -> Option<(usize, usize)> {
        use ffi::*;
        let mut image = std::ptr::null_mut();
        if AImageReader_acquireLatestImage(self.reader, &mut image) != 0 || image.is_null() {
            return None;
        }
        let (mut w, mut h, mut stride, mut len) = (0, 0, 0, 0);
        let mut data = std::ptr::null_mut();
        let read = AImage_getWidth(image, &mut w) == 0
            && AImage_getHeight(image, &mut h) == 0
            && AImage_getPlaneRowStride(image, 0, &mut stride) == 0
            && AImage_getPlaneData(image, 0, &mut data, &mut len) == 0
            && !data.is_null()
            && w > 0
            && h > 0
            && stride >= w
            && (len as i64) >= (stride as i64) * (h as i64 - 1) + w as i64;
        let size = if read {
            let (w, h, stride) = (w as usize, h as usize, stride as usize);
            let plane = std::slice::from_raw_parts(data, len as usize);
            grey.clear();
            for row in 0..h {
                grey.extend_from_slice(&plane[row * stride..row * stride + w]);
            }
            Some((w, h))
        } else {
            None
        };
        AImage_delete(image);
        size
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        use ffi::*;
        // SAFETY: each freed once, the session before the device and the
        // reader after both, only what was made.
        unsafe {
            if !self.session.is_null() {
                ACameraCaptureSession_stopRepeating(self.session);
                ACameraCaptureSession_close(self.session);
            }
            if !self.device.is_null() {
                ACameraDevice_close(self.device);
            }
            if !self.request.is_null() {
                ACaptureRequest_free(self.request);
            }
            if !self.target.is_null() {
                ACameraOutputTarget_free(self.target);
            }
            if !self.output.is_null() {
                ACaptureSessionOutput_free(self.output);
            }
            if !self.container.is_null() {
                ACaptureSessionOutputContainer_free(self.container);
            }
            if !self.reader.is_null() {
                AImageReader_delete(self.reader);
            }
            if !self.manager.is_null() {
                ACameraManager_delete(self.manager);
            }
        }
    }
}

/// The parts of camera2ndk (API 24) and mediandk's image reader used here,
/// as the NDK's headers declare them.
#[allow(non_camel_case_types, non_snake_case, clippy::upper_case_acronyms)]
mod ffi {
    use std::ffi::{c_char, c_int, c_void};

    macro_rules! opaque {
        ($($name:ident),*) => { $( #[repr(C)] pub struct $name { _p: [u8; 0] } )* };
    }
    opaque!(
        ACameraManager,
        ACameraMetadata,
        ACameraDevice,
        ACaptureRequest,
        ACameraOutputTarget,
        ACaptureSessionOutputContainer,
        ACaptureSessionOutput,
        ACameraCaptureSession,
        AImageReader,
        AImage,
        ANativeWindow
    );

    pub const AIMAGE_FORMAT_YUV_420_888: i32 = 0x23;
    pub const TEMPLATE_PREVIEW: c_int = 1;
    /// ACAMERA_LENS_START (ACAMERA_LENS = 8, shifted 16) + 5.
    pub const ACAMERA_LENS_FACING: u32 = (8 << 16) + 5;
    pub const ACAMERA_LENS_FACING_BACK: u8 = 1;

    #[repr(C)]
    pub struct ACameraIdList {
        pub num_cameras: c_int,
        pub camera_ids: *const *const c_char,
    }

    #[repr(C)]
    pub struct ACameraMetadata_const_entry {
        pub tag: u32,
        pub kind: u8,
        pub count: u32,
        /// (A union of pointers, one per value type.)
        pub data: *const c_void,
    }

    #[repr(C)]
    pub struct ACameraDevice_StateCallbacks {
        pub context: *mut c_void,
        pub on_disconnected: extern "C" fn(*mut c_void, *mut ACameraDevice),
        pub on_error: extern "C" fn(*mut c_void, *mut ACameraDevice, c_int),
    }

    #[repr(C)]
    pub struct ACameraCaptureSession_stateCallbacks {
        pub context: *mut c_void,
        pub on_closed: extern "C" fn(*mut c_void, *mut ACameraCaptureSession),
        pub on_ready: extern "C" fn(*mut c_void, *mut ACameraCaptureSession),
        pub on_active: extern "C" fn(*mut c_void, *mut ACameraCaptureSession),
    }

    #[link(name = "camera2ndk")]
    unsafe extern "C" {
        pub fn ACameraManager_create() -> *mut ACameraManager;
        pub fn ACameraManager_delete(manager: *mut ACameraManager);
        pub fn ACameraManager_getCameraIdList(manager: *mut ACameraManager, list: *mut *mut ACameraIdList) -> c_int;
        pub fn ACameraManager_deleteCameraIdList(list: *mut ACameraIdList);
        pub fn ACameraManager_getCameraCharacteristics(manager: *mut ACameraManager, id: *const c_char, meta: *mut *mut ACameraMetadata) -> c_int;
        pub fn ACameraMetadata_getConstEntry(meta: *const ACameraMetadata, tag: u32, entry: *mut ACameraMetadata_const_entry) -> c_int;
        pub fn ACameraMetadata_free(meta: *mut ACameraMetadata);
        pub fn ACameraManager_openCamera(
            manager: *mut ACameraManager,
            id: *const c_char,
            callbacks: *mut ACameraDevice_StateCallbacks,
            device: *mut *mut ACameraDevice,
        ) -> c_int;
        pub fn ACameraDevice_close(device: *mut ACameraDevice) -> c_int;
        pub fn ACameraDevice_createCaptureRequest(device: *const ACameraDevice, template: c_int, request: *mut *mut ACaptureRequest) -> c_int;
        pub fn ACaptureRequest_free(request: *mut ACaptureRequest);
        pub fn ACaptureRequest_addTarget(request: *mut ACaptureRequest, target: *const ACameraOutputTarget) -> c_int;
        pub fn ACameraOutputTarget_create(window: *mut ANativeWindow, target: *mut *mut ACameraOutputTarget) -> c_int;
        pub fn ACameraOutputTarget_free(target: *mut ACameraOutputTarget);
        pub fn ACaptureSessionOutputContainer_create(container: *mut *mut ACaptureSessionOutputContainer) -> c_int;
        pub fn ACaptureSessionOutputContainer_free(container: *mut ACaptureSessionOutputContainer);
        pub fn ACaptureSessionOutputContainer_add(container: *mut ACaptureSessionOutputContainer, output: *const ACaptureSessionOutput) -> c_int;
        pub fn ACaptureSessionOutput_create(window: *mut ANativeWindow, output: *mut *mut ACaptureSessionOutput) -> c_int;
        pub fn ACaptureSessionOutput_free(output: *mut ACaptureSessionOutput);
        pub fn ACameraDevice_createCaptureSession(
            device: *mut ACameraDevice,
            outputs: *const ACaptureSessionOutputContainer,
            callbacks: *const ACameraCaptureSession_stateCallbacks,
            session: *mut *mut ACameraCaptureSession,
        ) -> c_int;
        pub fn ACameraCaptureSession_setRepeatingRequest(
            session: *mut ACameraCaptureSession,
            callbacks: *mut c_void,
            count: c_int,
            requests: *mut *mut ACaptureRequest,
            sequence: *mut c_int,
        ) -> c_int;
        pub fn ACameraCaptureSession_stopRepeating(session: *mut ACameraCaptureSession) -> c_int;
        pub fn ACameraCaptureSession_close(session: *mut ACameraCaptureSession);
    }

    #[link(name = "mediandk")]
    unsafe extern "C" {
        pub fn AImageReader_new(width: i32, height: i32, format: i32, max_images: i32, reader: *mut *mut AImageReader) -> c_int;
        pub fn AImageReader_delete(reader: *mut AImageReader);
        pub fn AImageReader_getWindow(reader: *mut AImageReader, window: *mut *mut ANativeWindow) -> c_int;
        pub fn AImageReader_acquireLatestImage(reader: *mut AImageReader, image: *mut *mut AImage) -> c_int;
        pub fn AImage_delete(image: *mut AImage);
        pub fn AImage_getWidth(image: *const AImage, width: *mut i32) -> c_int;
        pub fn AImage_getHeight(image: *const AImage, height: *mut i32) -> c_int;
        pub fn AImage_getPlaneRowStride(image: *const AImage, plane: c_int, stride: *mut i32) -> c_int;
        pub fn AImage_getPlaneData(image: *const AImage, plane: c_int, data: *mut *mut u8, len: *mut c_int) -> c_int;
    }
}
