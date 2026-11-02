use js_sys::{Array, Object, Reflect};
use wasm_bindgen::{JsCast, JsValue};

pub(super) fn object() -> Object {
    Object::new()
}

pub(super) fn dictionary<T: JsCast>() -> T {
    object().unchecked_into()
}

pub(super) fn set(object: &Object, name: &str, value: &JsValue) {
    Reflect::set(object.as_ref(), &JsValue::from_str(name), value)
        .unwrap_or_else(|error| panic!("set WebGPU descriptor field {name}: {error:?}"));
}

pub(super) fn set_ref<T: AsRef<JsValue>>(object: &Object, name: &str, value: &T) {
    set(object, name, value.as_ref());
}

pub(super) fn set_str(object: &Object, name: &str, value: &str) {
    set(object, name, &JsValue::from_str(value));
}

pub(super) fn set_bool(object: &Object, name: &str, value: bool) {
    set(object, name, &JsValue::from_bool(value));
}

pub(super) fn set_u32(object: &Object, name: &str, value: u32) {
    set(object, name, &JsValue::from_f64(f64::from(value)));
}

pub(super) fn set_i32(object: &Object, name: &str, value: i32) {
    set(object, name, &JsValue::from_f64(f64::from(value)));
}

pub(super) fn set_f64(object: &Object, name: &str, value: f64) {
    set(object, name, &JsValue::from_f64(value));
}

pub(super) fn set_array(object: &Object, name: &str, value: &Array) {
    set(object, name, value.as_ref());
}

pub(super) fn push_value(array: &Array, value: &JsValue) {
    array.push(value);
}

pub(super) fn js_error(value: JsValue) -> String {
    value.as_string().unwrap_or_else(|| format!("{value:?}"))
}
