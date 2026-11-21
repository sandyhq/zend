//! Android: a banner through the system's `NotificationManager`, on a
//! channel of Zend's own ("Sessions", made the first time), with the app's
//! icon; a tap brings the app forward (its launch intent). Called through
//! the Java VM and the `Application` android-activity hands ndk-context:
//! no Java code of the app's own. Android 13 and later show it only once
//! the person allowed notifications (POST_NOTIFICATIONS, asked as the app
//! starts); before that the system drops it.

use jni::objects::{JObject, JValue};
use jni::JNIEnv;
use std::sync::atomic::{AtomicI32, Ordering};

const CHANNEL: &str = "zend-sessions";
/// `NotificationManager.IMPORTANCE_DEFAULT`.
const IMPORTANCE_DEFAULT: i32 = 3;
/// `PendingIntent.FLAG_IMMUTABLE | FLAG_UPDATE_CURRENT`.
const PENDING_FLAGS: i32 = 0x0400_0000 | 0x0800_0000;

/// Each banner's id: a new one each, so one doesn't replace another.
static NEXT_ID: AtomicI32 = AtomicI32::new(1);

pub(crate) fn post(title: &str, body: &str, _target: Option<&str>) -> Result<(), String> {
    let context = ndk_context::android_context();
    if context.vm().is_null() || context.context().is_null() {
        return Err("no Android context yet".into());
    }
    // SAFETY: ndk-context's VM is the process's; its context a global
    // reference to the Application that android-activity never deletes.
    let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }.map_err(|e| e.to_string())?;
    let app = unsafe { JObject::from_raw(context.context().cast()) };
    let mut env = vm.attach_current_thread_permanently().map_err(|e| e.to_string())?;
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let result = env.with_local_frame(32, |env| show(env, &app, id, title, body));
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
        return Err("a Java exception".into());
    }
    result.map_err(|e| e.to_string())
}

fn show(env: &mut JNIEnv, app: &JObject, id: i32, title: &str, body: &str) -> jni::errors::Result<()> {
    let service = env.new_string("notification")?;
    let manager = env
        .call_method(app, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&service)])?
        .l()?;
    // The channel (API 26+; again is a no-op).
    let channel_id = env.new_string(CHANNEL)?;
    let channel_name = env.new_string("Sessions")?;
    let channel = env.new_object(
        "android/app/NotificationChannel",
        "(Ljava/lang/String;Ljava/lang/CharSequence;I)V",
        &[JValue::Object(&channel_id), JValue::Object(&channel_name), JValue::Int(IMPORTANCE_DEFAULT)],
    )?;
    env.call_method(&manager, "createNotificationChannel", "(Landroid/app/NotificationChannel;)V", &[JValue::Object(&channel)])?;
    // The app's icon, and its launch intent for a tap.
    let info = env.call_method(app, "getApplicationInfo", "()Landroid/content/pm/ApplicationInfo;", &[])?.l()?;
    // (The app's icon, or the system's chat glyph while it has none: a
    // banner without one is refused, IllegalArgumentException.)
    let icon = match env.get_field(&info, "icon", "I")?.i()? {
        0 => env.get_static_field("android/R$drawable", "stat_notify_chat", "I")?.i()?,
        icon => icon,
    };
    let package = env.call_method(app, "getPackageName", "()Ljava/lang/String;", &[])?.l()?;
    let packages = env.call_method(app, "getPackageManager", "()Landroid/content/pm/PackageManager;", &[])?.l()?;
    let launch = env
        .call_method(&packages, "getLaunchIntentForPackage", "(Ljava/lang/String;)Landroid/content/Intent;", &[JValue::Object(&package)])?
        .l()?;
    let builder = env.new_object(
        "android/app/Notification$Builder",
        "(Landroid/content/Context;Ljava/lang/String;)V",
        &[JValue::Object(app), JValue::Object(&channel_id)],
    )?;
    let title = env.new_string(title)?;
    let body = env.new_string(body)?;
    let b = "Landroid/app/Notification$Builder;";
    env.call_method(&builder, "setContentTitle", format!("(Ljava/lang/CharSequence;){b}"), &[JValue::Object(&title)])?;
    env.call_method(&builder, "setContentText", format!("(Ljava/lang/CharSequence;){b}"), &[JValue::Object(&body)])?;
    env.call_method(&builder, "setSmallIcon", format!("(I){b}"), &[JValue::Int(icon)])?;
    env.call_method(&builder, "setAutoCancel", format!("(Z){b}"), &[JValue::Bool(1)])?;
    if !launch.is_null() {
        let pending = env
            .call_static_method(
                "android/app/PendingIntent",
                "getActivity",
                "(Landroid/content/Context;ILandroid/content/Intent;I)Landroid/app/PendingIntent;",
                &[JValue::Object(app), JValue::Int(0), JValue::Object(&launch), JValue::Int(PENDING_FLAGS)],
            )?
            .l()?;
        env.call_method(&builder, "setContentIntent", format!("(Landroid/app/PendingIntent;){b}"), &[JValue::Object(&pending)])?;
    }
    let notification = env.call_method(&builder, "build", "()Landroid/app/Notification;", &[])?.l()?;
    env.call_method(&manager, "notify", "(ILandroid/app/Notification;)V", &[JValue::Int(id), JValue::Object(&notification)])?;
    Ok(())
}
