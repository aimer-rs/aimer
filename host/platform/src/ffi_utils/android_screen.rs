// use jni::JNIEnv;
// use jni::objects::JObject;
//
// pub fn get_screen_size(env: &JNIEnv, context: JObject) -> (f32, f32) {
//     // Get Resources
//     let resources = env
//         .call_method(context, "getResources",
// "()Landroid/content/res/Resources;", &[])         .unwrap()
//         .l()
//         .unwrap();
//
//     // Get DisplayMetrics
//     let metrics = env
//         .call_method(resources, "getDisplayMetrics",
// "()Landroid/util/DisplayMetrics;", &[])         .unwrap()
//         .l()
//         .unwrap();
//
//     // widthPixels
//     let width_pixels = env
//         .get_field(metrics, "widthPixels", "I")
//         .unwrap()
//         .i()
//         .unwrap() as f32;
//
//     // heightPixels
//     let height_pixels = env
//         .get_field(metrics, "heightPixels", "I")
//         .unwrap()
//         .i()
//         .unwrap() as f32;
//
//     // xdpi
//     let xdpi = env
//         .get_field(metrics, "xdpi", "F")
//         .unwrap()
//         .f()
//         .unwrap();
//
//     // ydpi
//     let ydpi = env
//         .get_field(metrics, "ydpi", "F")
//         .unwrap()
//         .f()
//         .unwrap();
//
//     let width_inches = width_pixels / xdpi;
//     let height_inches = height_pixels / ydpi;
//
//     (width_inches, height_inches)
// }
