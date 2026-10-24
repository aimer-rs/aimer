#[cfg(feature = "hot-reload")]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    #[test]
    fn derived_value_is_a_bounded_property_codec() {
        run_fixture(
            "guest",
            &["test", "--quiet", "--features", "portable-guest"],
        );
    }

    #[test]
    fn derived_value_compiles_without_guest_feature() {
        run_fixture("host", &["test", "--quiet", "--no-default-features"]);
    }

    #[test]
    fn derived_value_keeps_the_same_wire_contract_with_serde_derives() {
        run_fixture(
            "serde",
            &[
                "test",
                "--quiet",
                "--no-default-features",
                "--features",
                "serde",
            ],
        );
    }

    fn run_fixture(name: &str, args: &[&str]) {
        let fixture = fixture_root(name);
        if fixture.exists() {
            fs::remove_dir_all(&fixture).unwrap();
        }
        fs::create_dir_all(fixture.join("src")).unwrap();
        fs::write(fixture.join("Cargo.toml"), fixture_manifest()).unwrap();
        fs::write(fixture.join("src/lib.rs"), FIXTURE_SOURCE).unwrap();

        let output = Command::new(env!("CARGO"))
            .args(args)
            .arg("--manifest-path")
            .arg(fixture.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", fixture.join("target"))
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "portable value fixture failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    fn fixture_manifest() -> String {
        let macro_crate = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = macro_crate.join("../..");
        format!(
            r#"[package]
name = "portable_value_fixture"
version = "0.0.0"
edition = "2024"

[workspace]

[features]
portable-guest = ["aimer_widget/portable-guest"]
serde = ["dep:serde"]

[dependencies]
aimer_macro = {{ path = {:?} }}
aimer_widget = {{ path = {:?} }}
aimer_anteros = {{ path = {:?} }}
aimer_provider = {{ path = {:?} }}
serde = {{ version = "1.0.228", features = ["derive"], optional = true }}
"#,
            macro_crate,
            workspace_root.join("crates/aimer_widget"),
            workspace_root.join("aimer_anteros"),
            workspace_root.join("crates/aimer_provider"),
        )
    }

    fn fixture_root(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/aimer_macro_portable_value_{name}"))
    }

    const FIXTURE_SOURCE: &str = r#"
use aimer_anteros::Version;
#[cfg(feature = "portable-guest")]
use aimer_anteros::{ModelLimits, PropertyId, PropertyValue, WidgetDocument, WidgetNode,
    WidgetProperty, WidgetSchemaId};
use aimer_macro::PortableValue;
use aimer_provider::PortableProviderCodec;
use aimer_widget::portable::PortableValue as PortableValueTrait;
#[cfg(feature = "portable-guest")]
use aimer_widget::portable::{
    PortableBuildContext, PortableEncodeProperty, PortableLimits, PortableMaterializeProperty,
    PortableProperty, PortableWidgetLimits, SourceFingerprint, StableId128,
};
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, LinkedList, VecDeque};

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:portable_value_fixture::BorderRadius",
    version = "1.0",
    max_encoded_bytes = 64,
)]
struct BorderRadius {
    #[portable_value(order = 1)]
    top_left: u16,
    #[portable_value(order = 0)]
    bottom_right: u16,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PortableValue)]
#[portable_value(
    id = "aimer.value:portable_value_fixture::CollectionValue",
    version = "1.0",
    max_encoded_bytes = 512,
    max_depth = 16,
    max_entries = 64,
    max_key_bytes = 8,
    max_value_bytes = 16,
    max_reconstruction_work = 128,
)]
struct CollectionValue {
    optional: Option<Box<u32>>,
    result: Result<u8, String>,
    array: [u16; 2],
    vector: Vec<String>,
    deque: VecDeque<u8>,
    list: LinkedList<i32>,
    map: BTreeMap<String, u8>,
    set: BTreeSet<u8>,
    heap: BinaryHeap<u8>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(id = "aimer.value:portable_value_fixture::EmptyValue", max_encoded_bytes = 4)]
struct EmptyValue;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(id = "aimer.value:portable_value_fixture::TinyValue", max_encoded_bytes = 8)]
struct TinyValue {
    text: String,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:portable_value_fixture::DeepValue",
    max_encoded_bytes = 32,
    max_depth = 1,
)]
struct DeepValue {
    nested: Option<Box<u8>>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:portable_value_fixture::Shape",
    version = "2.3",
    max_encoded_bytes = 64,
)]
enum Shape {
    #[portable_value(tag = 7)]
    Circle { radius: u16 },
    #[portable_value(tag = 2)]
    Empty,
    #[portable_value(tag = 9)]
    Pair(u8, u8),
}

#[cfg(feature = "portable-guest")]
fn context() -> PortableBuildContext {
    PortableBuildContext::new(
        1,
        0,
        PortableWidgetLimits::new(4, 16, 4, 4, 64, 4_096).with_max_blob_bytes(128),
        PortableLimits::new(8, 64, 64, 128, 1_024),
    )
    .unwrap()
}

#[test]
#[cfg(feature = "portable-guest")]
fn derived_value_has_one_deterministic_schema_and_blob_path() {
    assert_eq!(
        <BorderRadius as PortableValueTrait>::SCHEMA.canonical_name(),
        "aimer.value:portable_value_fixture::BorderRadius",
    );
    assert_eq!(
        <BorderRadius as PortableValueTrait>::SCHEMA.version(),
        Version::new(1, 0),
    );
    assert_eq!(
        <BorderRadius as PortableProperty>::REFLECTION.value_schema(),
        Some(<BorderRadius as PortableValueTrait>::SCHEMA),
    );

    let value = BorderRadius { top_left: 12, bottom_right: 34 };
    let bytes = value.encode_value().unwrap();
    assert_eq!(
        bytes,
        vec![1, 0, 0, 0, 34, 0, 12, 0],
    );

    let mut context = context();
    let property = value.clone().encode_property(&mut context).unwrap();
    assert_eq!(property, PropertyValue::BlobRef(0));
    assert_eq!(
        value.clone().encode_property(&mut context).unwrap(),
        PropertyValue::BlobRef(0),
    );
    let node = context
        .push_node(
            WidgetSchemaId::new(1),
            Version::new(1, 0),
            None,
            SourceFingerprint::new(StableId128::from_u128(1)),
            &[WidgetProperty::new(PropertyId::new(7), property)],
            &[],
        )
        .unwrap();
    let graph = context.finish_graph(node).unwrap();
    assert_eq!(graph.blob(0), Some(bytes.as_slice()));

    let blobs = [bytes.as_slice()];
    let image = WidgetDocument::new(
        1,
        0,
        0,
        &[WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0))],
        &[],
        &blobs,
    )
    .encode(ModelLimits::new(4_096, 16, 64, 128))
    .unwrap();
    let document = aimer_anteros::WidgetDocumentView::decode(
        &image,
        ModelLimits::new(4_096, 16, 64, 128),
    )
    .unwrap();
    let decoded = BorderRadius::from_awir(
        &document,
        PropertyId::new(7),
        PropertyValue::BlobRef(0),
    )
    .unwrap();
    assert_eq!(decoded, value);
}

#[test]
fn derived_values_cover_ordered_collections_and_explicit_enum_tags() {
    assert_eq!(<CollectionValue as PortableValueTrait>::MAX_KEY_BYTES, 8);
    assert_eq!(<CollectionValue as PortableValueTrait>::MAX_VALUE_BYTES, 16);
    assert_eq!(
        <CollectionValue as PortableValueTrait>::MAX_RECONSTRUCTION_WORK,
        128,
    );

    let mut map = BTreeMap::new();
    map.insert("b".to_owned(), 2);
    map.insert("a".to_owned(), 1);
    let mut list = LinkedList::new();
    list.push_back(-3);
    list.push_back(8);
    let value = CollectionValue {
        optional: Some(Box::new(9)),
        result: Err("portable error".to_owned()),
        array: [4, 5],
        vector: vec!["one".to_owned(), "two".to_owned()],
        deque: VecDeque::from([6, 7]),
        list,
        map,
        set: BTreeSet::from([3, 1]),
        heap: BinaryHeap::from([2, 9, 4]),
    };
    let bytes = value.encode_value().unwrap();
    let decoded = CollectionValue::decode_value(&bytes, Version::new(1, 0)).unwrap();
    assert_eq!(&decoded.optional, &value.optional);
    assert_eq!(&decoded.result, &value.result);
    assert_eq!(&decoded.array, &value.array);
    assert_eq!(&decoded.vector, &value.vector);
    assert_eq!(&decoded.deque, &value.deque);
    assert_eq!(&decoded.list, &value.list);
    assert_eq!(&decoded.map, &value.map);
    assert_eq!(&decoded.set, &value.set);
    assert_eq!(decoded.heap.into_sorted_vec(), value.heap.clone().into_sorted_vec());

    for shape in [Shape::Empty, Shape::Circle { radius: 11 }, Shape::Pair(1, 2)] {
        let bytes = shape.encode_value().unwrap();
        assert_eq!(Shape::decode_value(&bytes, Version::new(2, 3)).unwrap(), shape);
    }
    assert_eq!(<Shape as PortableValueTrait>::VARIANTS[0].tag(), 2);
    assert_eq!(<Shape as PortableValueTrait>::VARIANTS[1].tag(), 7);
    assert_eq!(<Shape as PortableValueTrait>::VARIANTS[2].tag(), 9);
}

#[test]
fn derived_values_reject_empty_budget_overflows_and_excess_depth() {
    let empty = EmptyValue;
    let bytes = empty.encode_value().unwrap();
    assert_eq!(bytes, vec![1, 0, 0, 0]);
    assert_eq!(EmptyValue::decode_value(&bytes, Version::new(1, 0)).unwrap(), empty);

    assert!(TinyValue { text: "long".to_owned() }.encode_value().is_err());
    assert!(DeepValue { nested: Some(Box::new(1)) }.encode_value().is_err());
}

#[test]
fn derived_value_provider_codec_reuses_the_same_wire_contract() {
    let value = BorderRadius { top_left: 3, bottom_right: 8 };
    let codec = PortableProviderCodec::<BorderRadius>::from_portable_value();
    assert_eq!(codec.schema(), <BorderRadius as PortableValueTrait>::SCHEMA);
    let bytes = codec.encode(&value).unwrap();
    assert_eq!(bytes, value.encode_value().unwrap());
    assert_eq!(codec.decode(&bytes, Version::new(1, 0)).unwrap(), value);
    assert!(codec.decode(&bytes, Version::new(2, 0)).is_err());
}

#[test]
fn derived_value_rejects_invalid_versions_and_trailing_payload() {
    let value = BorderRadius { top_left: 3, bottom_right: 8 };
    let mut bytes = value.encode_value().unwrap();
    assert!(BorderRadius::decode_value(&bytes, Version::new(2, 0)).is_err());
    bytes[0] = 2;
    assert!(BorderRadius::decode_value(&bytes, Version::new(1, 0)).is_err());
    let mut trailing = value.encode_value().unwrap();
    trailing.push(0);
    assert!(BorderRadius::decode_value(&trailing, Version::new(1, 0)).is_err());
}
"#;

    #[test]
    fn derive_rejects_raw_hash_collections_without_an_adapter() {
        let fixture = fixture_root("hash_map");
        if fixture.exists() {
            fs::remove_dir_all(&fixture).unwrap();
        }
        fs::create_dir_all(fixture.join("src")).unwrap();
        fs::write(fixture.join("Cargo.toml"), fixture_manifest()).unwrap();
        fs::write(
            fixture.join("src/lib.rs"),
            r#"
use std::collections::HashMap;
use aimer_macro::PortableValue;

#[derive(PortableValue)]
#[portable_value(id = "aimer.value:raw", max_encoded_bytes = 32)]
struct Raw {
    values: HashMap<String, u8>,
}
"#,
        )
        .unwrap();
        let output = Command::new(env!("CARGO"))
            .args(["check", "--quiet"])
            .arg("--manifest-path")
            .arg(fixture.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", fixture.join("target"))
            .output()
            .unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "raw HashMap unexpectedly compiled"
        );
        assert!(diagnostic.contains("CanonicalHashMap"), "{diagnostic}");
    }
}
