//! Android browser handoff through the original application Context, without a desktop subprocess.

use jni::{
    JavaVM, jni_sig, jni_str,
    objects::{JObject, JValue},
};

/// Starts Android's ACTION_VIEW handler; failure leaves the existing manual sign-in recovery visible.
pub(super) fn open_url(url: &str) -> bool {
    let context = ndk_context::android_context();
    // SAFETY: NativeActivity initializes ndk-context with its live VM and global Context reference.
    let vm = unsafe { JavaVM::from_raw(context.vm().cast()) };
    vm.attach_current_thread_for_scope(|env| -> jni::errors::Result<()> {
        // SAFETY: ndk-context retains this reference while the NativeActivity is alive.
        let activity_ptr = context.context().cast();
        let activity = unsafe { env.as_cast_raw::<JObject>(&activity_ptr)? };
        let context = env
            .call_method(
                &activity,
                jni_str!("getApplicationContext"),
                jni_sig!("()Landroid/content/Context;"),
                &[],
            )?
            .l()?;
        let uri = env.new_string(url)?;
        let uri = env
            .call_static_method(
                jni_str!("android/net/Uri"),
                jni_str!("parse"),
                jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                &[JValue::Object(&uri)],
            )?
            .l()?;
        let action = env
            .get_static_field(
                jni_str!("android/content/Intent"),
                jni_str!("ACTION_VIEW"),
                jni_sig!("Ljava/lang/String;"),
            )?
            .l()?;
        let intent = env.new_object(
            jni_str!("android/content/Intent"),
            jni_sig!("(Ljava/lang/String;Landroid/net/Uri;)V"),
            &[JValue::Object(&action), JValue::Object(&uri)],
        )?;
        let flags = env
            .get_static_field(
                jni_str!("android/content/Intent"),
                jni_str!("FLAG_ACTIVITY_NEW_TASK"),
                jni_sig!("I"),
            )?
            .i()?;
        env.call_method(
            &intent,
            jni_str!("addFlags"),
            jni_sig!("(I)Landroid/content/Intent;"),
            &[JValue::Int(flags)],
        )?;
        env.call_method(
            context,
            jni_str!("startActivity"),
            jni_sig!("(Landroid/content/Intent;)V"),
            &[JValue::Object(&intent)],
        )?;
        Ok(())
    })
    .is_ok()
}
