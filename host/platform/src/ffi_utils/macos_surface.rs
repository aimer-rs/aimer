use objc::runtime::{BOOL, Class, Object, YES};
use objc::{msg_send, sel, sel_impl};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MetalLayerLocation {
    Root,
    Sublayer(usize),
}

fn metal_layer_location<I>(
    root_is_metal: bool,
    sublayers_are_metal: impl FnOnce() -> Option<I>,
) -> Option<MetalLayerLocation>
where
    I: IntoIterator<Item = bool>,
{
    if root_is_metal {
        return Some(MetalLayerLocation::Root);
    }

    sublayers_are_metal()?
        .into_iter()
        .enumerate()
        .filter_map(|(index, is_metal)| is_metal.then_some(index))
        .last()
        .map(MetalLayerLocation::Sublayer)
}

#[allow(unexpected_cfgs)]
pub fn enable_transactional_surface_presentation(window: &Window) -> bool {
    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return false;
    };

    // SAFETY: The AppKit handle originates from winit and this function is
    // called on winit's main event-loop thread immediately after the renderer
    // creates its CAMetalLayer. Potentially absent layers are checked before
    // sending messages to them.
    unsafe {
        let view = appkit.ns_view.as_ptr().cast::<Object>();
        let root_layer: *mut Object = msg_send![view, layer];
        if root_layer.is_null() {
            return false;
        }

        let Some(metal_layer_class) = Class::get("CAMetalLayer") else {
            return false;
        };

        let root_is_metal: BOOL = msg_send![root_layer, isKindOfClass: metal_layer_class];
        let mut layers = Vec::new();
        let location = metal_layer_location(root_is_metal == YES, || {
            let sublayers: *mut Object = msg_send![root_layer, sublayers];
            if sublayers.is_null() {
                return None;
            }

            let count: usize = msg_send![sublayers, count];
            let mut layer_kinds = Vec::with_capacity(count);
            for index in 0..count {
                let layer: *mut Object = msg_send![sublayers, objectAtIndex: index];
                let is_metal: BOOL = msg_send![layer, isKindOfClass: metal_layer_class];
                layers.push(layer);
                layer_kinds.push(is_metal == YES);
            }
            Some(layer_kinds)
        });

        let layer = match location {
            Some(MetalLayerLocation::Root) => root_layer,
            Some(MetalLayerLocation::Sublayer(index)) => layers[index],
            None => return false,
        };
        let _: () = msg_send![layer, setPresentsWithTransaction: YES];
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_metal_layer_takes_priority() {
        assert_eq!(
            metal_layer_location(true, || -> Option<[bool; 0]> {
                panic!("root layer must short-circuit sublayer lookup")
            }),
            Some(MetalLayerLocation::Root)
        );
    }

    #[test]
    fn metal_sublayer_is_selected() {
        assert_eq!(
            metal_layer_location(false, || Some([true, false, true])),
            Some(MetalLayerLocation::Sublayer(2))
        );
    }

    #[test]
    fn missing_sublayers_are_reported_without_querying_a_null_array() {
        assert_eq!(metal_layer_location(false, || None::<[bool; 0]>), None);
    }

    #[test]
    fn missing_metal_layer_is_reported() {
        assert_eq!(metal_layer_location(false, || Some([false, false])), None);
    }
}
