const ABI_VERSION: u64 = 1_u64 << 32;
const OUTPUT_POINTER: i64 = 4_096;
const AWIR_POINTER: i32 = 1_024;
const ASTA_POINTER: i32 = 2_048;
const AMNF_POINTER: i32 = 3_072;
const MIGRATED_ASTA_POINTER: i32 = 8_192;
const CLEANUP_MARKER_POINTER: i64 = 512;

#[derive(Clone, Copy)]
struct CallbackGuestShape {
    include_manifest: bool,
    manifest_type: u8,
    unexpected_export: bool,
    start: bool,
    capability_import: bool,
    capability_build: bool,
    wrong_capability_signature: bool,
    strip_vector_prefix: bool,
    capability_request_length: i32,
    dispatch_async_event: bool,
    async_state_update: bool,
    mutate_widget_source: bool,
    dispatch_widget_output: bool,
    partial_widget_output: bool,
    migration_export: bool,
    upgraded_state: bool,
    migration_trap: bool,
    migration_infinite: bool,
}

impl CallbackGuestShape {
    const VALID: Self = Self {
        include_manifest: true,
        manifest_type: 1,
        unexpected_export: false,
        start: false,
        capability_import: false,
        capability_build: false,
        wrong_capability_signature: false,
        strip_vector_prefix: false,
        capability_request_length: 0,
        dispatch_async_event: false,
        async_state_update: false,
        mutate_widget_source: false,
        dispatch_widget_output: false,
        partial_widget_output: false,
        migration_export: false,
        upgraded_state: false,
        migration_trap: false,
        migration_infinite: false,
    };
}
#[rustfmt::skip]
pub const AWIR: &[u8] = &[
    b'A', b'W', b'I', b'R',
    2, 0, 0, 0,
    11, 0, 0, 0, 0, 0, 0, 0,
    13, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0,
    1, 0, 0, 0,
    0, 0, 0, 0,
    0, 0, 0, 0,
    0, 0, 0, 0,
    0, 0, 0, 0,
    0, 0, 0, 0,
    0, 0, 0, 0,
    0, 0, 0, 0,
    128, 0, 0, 0,
    7, 0, 0, 0, 0, 0, 0, 0,
    1, 0, 2, 0,
    0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0,
    0, 0, 0, 0,
    0, 0, 0, 0,
    0, 0, 0, 0,
    0, 0, 0, 0,
    0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
];
#[rustfmt::skip]
const ASTA_TEMPLATE: &[u8] = &[
    b'A', b'S', b'T', b'A',
    1, 0, 0, 0,
    0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
    0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
    7, 0, 0, 0, 0, 0, 0, 0,
    1, 0, 0, 0,
    1, 0, 0, 0,
    97, 0, 0, 0,
    0, 0, 0, 0,
    0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
    0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
    0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30,
    0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30,
    2, 0, 1, 0,
    1, 0, 0, 0,
    0, 0, 0, 0,
    1, 0, 0, 0,
    0,
];
#[rustfmt::skip]
const AMNF: &[u8] = &[
    b'A', b'M', b'N', b'F',
    1, 0, 0, 0,
    1, 0, 0, 0, 0, 0, 0, 0,
    1, 0, 0, 0, 0, 0, 0, 0,
    2, 0, 0, 0,
    2, 0, 0, 0,
    1, 0, 0, 0,
    0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
    0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
    0, 0, 0, 0,
    64, 0, 0, 0,
    0, 0, 0, 0,
];
#[rustfmt::skip]
const CAPABILITY_AMNF: &[u8] = &[
    b'A', b'M', b'N', b'F',
    1, 0, 0, 0,
    1, 0, 0, 0, 0, 0, 0, 0,
    1, 0, 0, 0, 0, 0, 0, 0,
    2, 0, 0, 0,
    2, 0, 0, 0,
    1, 0, 0, 0,
    0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
    0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
    1, 0, 0, 0,
    120, 0, 0, 0,
    0, 0, 0, 0,
    0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
    0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
    1, 0, 0, 0,
    1, 0, 0, 0,
    0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30,
    0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30,
    0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30,
    0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30,
];

pub fn build_guest() -> Vec<u8> {
    build_guest_with_abi(ABI_VERSION)
}

pub fn build_guest_with_abi(abi_version: u64) -> Vec<u8> {
    assemble_guest(abi_version, OUTPUT_POINTER, build_body())
}

pub fn diagnostic_build_guest() -> Vec<u8> {
    let diagnostic = aimer_anteros::GuestDiagnostic::new(
        aimer_anteros::GuestOperation::Build,
        aimer_anteros::GuestDiagnosticCategory::UnsupportedWidget,
        "widget has no guest lowering",
    )
    .with_widget("Container")
    .with_source(aimer_anteros::StableId128::from_bytes([0xCD; 16]))
    .encode()
    .unwrap();
    assemble_guest_with_diagnostic(ABI_VERSION, diagnostic)
}

pub fn malformed_diagnostic_build_guest() -> Vec<u8> {
    assemble_guest_with_diagnostic_bytes(ABI_VERSION, b"bad".to_vec())
}

pub fn oversized_diagnostic_build_guest() -> Vec<u8> {
    assemble_guest_with_diagnostic_bytes(
        ABI_VERSION,
        vec![0; aimer_anteros::MAX_GUEST_DIAGNOSTIC_BYTES + 1],
    )
}

pub fn callback_state_guest() -> Vec<u8> {
    assemble_callback_state_guest(OUTPUT_POINTER, 0, state_import_body())
}

pub fn capability_build_guest() -> Vec<u8> {
    assemble_callback_state_guest_with_bodies(
        i64_constant(OUTPUT_POINTER),
        vec![0, 0x41, 0, 0x0B],
        output_body(AMNF_POINTER, CAPABILITY_AMNF.len()),
        state_import_body(),
        CallbackGuestShape {
            capability_import: true,
            capability_build: true,
            ..CallbackGuestShape::VALID
        },
    )
}

#[allow(dead_code)]
pub fn capability_async_guest() -> Vec<u8> {
    let mut module = assemble_callback_state_guest_with_bodies(
        i64_constant(OUTPUT_POINTER),
        vec![0, 0x41, 0, 0x0B],
        output_body(AMNF_POINTER, CAPABILITY_AMNF.len()),
        state_import_body(),
        CallbackGuestShape {
            capability_import: true,
            capability_build: true,
            dispatch_async_event: true,
            async_state_update: true,
            ..CallbackGuestShape::VALID
        },
    );

    // The general fixture intentionally keeps its old hand-written AWIR so
    // tests can exercise malformed/legacy candidate handling. The capability
    // protocol proof needs a candidate that reaches the provider, though, so
    // give this one a current, minimal built-in Column root without changing
    // the shared fixture used by the other protocol tests.
    let awir_offset = module
        .windows(AWIR.len())
        .position(|window| window == AWIR)
        .expect("capability guest must contain its AWIR data segment");
    let awir = &mut module[awir_offset..awir_offset + AWIR.len()];
    awir[64..72].copy_from_slice(&[148, 26, 14, 85, 127, 94, 99, 1]);
    awir[72..76].copy_from_slice(&[1, 0, 0, 0]);
    module
}

#[allow(dead_code)]
pub fn capability_build_guest_with_contract(
    capability_id: [u8; 16],
    contract_fingerprint: [u8; 32],
) -> Vec<u8> {
    let mut module = assemble_callback_state_guest_with_bodies(
        i64_constant(OUTPUT_POINTER),
        vec![0, 0x41, 0, 0x0B],
        output_body(AMNF_POINTER, CAPABILITY_AMNF.len()),
        state_import_body(),
        CallbackGuestShape {
            capability_import: true,
            capability_build: true,
            strip_vector_prefix: true,
            ..CallbackGuestShape::VALID
        },
    );
    replace_last(&mut module, &[0x20; 16], &capability_id);
    replace_last(&mut module, &[0x30; 32], &contract_fingerprint);
    module
}

pub fn wrong_capability_import_signature_guest() -> Vec<u8> {
    assemble_callback_state_guest_with_bodies(
        i64_constant(OUTPUT_POINTER),
        vec![0, 0x41, 0, 0x0B],
        output_body(AMNF_POINTER, CAPABILITY_AMNF.len()),
        state_import_body(),
        CallbackGuestShape {
            capability_import: true,
            wrong_capability_signature: true,
            ..CallbackGuestShape::VALID
        },
    )
}

pub fn oversized_capability_request_guest() -> Vec<u8> {
    assemble_callback_state_guest_with_bodies(
        i64_constant(OUTPUT_POINTER),
        vec![0, 0x41, 0, 0x0B],
        output_body(AMNF_POINTER, CAPABILITY_AMNF.len()),
        state_import_body(),
        CallbackGuestShape {
            capability_import: true,
            capability_build: true,
            capability_request_length: 1_000_000,
            ..CallbackGuestShape::VALID
        },
    )
}

pub fn missing_manifest_guest() -> Vec<u8> {
    assemble_callback_state_guest_with_bodies(
        i64_constant(OUTPUT_POINTER),
        vec![0, 0x41, 0, 0x0B],
        manifest_body(),
        state_import_body(),
        CallbackGuestShape {
            include_manifest: false,
            ..CallbackGuestShape::VALID
        },
    )
}

pub fn wrong_manifest_signature_guest() -> Vec<u8> {
    assemble_callback_state_guest_with_bodies(
        i64_constant(OUTPUT_POINTER),
        vec![0, 0x41, 0, 0x0B],
        i64_constant(0),
        state_import_body(),
        CallbackGuestShape {
            manifest_type: 0,
            ..CallbackGuestShape::VALID
        },
    )
}

pub fn unexpected_export_guest() -> Vec<u8> {
    assemble_shaped_callback_guest(CallbackGuestShape {
        unexpected_export: true,
        ..CallbackGuestShape::VALID
    })
}

pub fn start_function_guest() -> Vec<u8> {
    assemble_shaped_callback_guest(CallbackGuestShape {
        start: true,
        ..CallbackGuestShape::VALID
    })
}

pub fn imported_memory_guest() -> Vec<u8> {
    let mut module = b"\0asm\x01\0\0\0".to_vec();
    let mut imports = vec![1];
    push_name(&mut imports, "environment");
    push_name(&mut imports, "memory");
    imports.extend_from_slice(&[2, 0, 1]);
    push_section(&mut module, 2, &imports);
    module
}

pub fn memory_growth_guest() -> Vec<u8> {
    assemble_i32_guest(
        "grow",
        Some(&[1, 0, 1]),
        None,
        vec![0, 0x41, 1, 0x40, 0, 0x0B],
    )
}

pub fn table_growth_guest() -> Vec<u8> {
    assemble_i32_guest(
        "grow",
        None,
        Some(&[1, 0x70, 0, 1]),
        vec![0, 0xD0, 0x70, 0x41, 1, 0xFC, 0x0F, 0, 0x0B],
    )
}

pub fn recursive_guest() -> Vec<u8> {
    assemble_i32_guest("recurse", None, None, vec![0, 0x10, 0, 0x0B])
}

pub fn simd_guest() -> Vec<u8> {
    let mut body = vec![0, 0xFD, 0x0C];
    body.extend_from_slice(&[0; 16]);
    body.extend_from_slice(&[0x1A, 0x41, 0, 0x0B]);
    assemble_i32_guest("simd", None, None, body)
}

pub fn widget_source_mutation_guest() -> Vec<u8> {
    assemble_shaped_callback_guest(CallbackGuestShape {
        mutate_widget_source: true,
        ..CallbackGuestShape::VALID
    })
}

pub fn callback_widget_output_guest() -> Vec<u8> {
    assemble_shaped_callback_guest(CallbackGuestShape {
        dispatch_widget_output: true,
        ..CallbackGuestShape::VALID
    })
}

pub fn callback_partial_widget_output_guest() -> Vec<u8> {
    assemble_shaped_callback_guest(CallbackGuestShape {
        dispatch_widget_output: true,
        partial_widget_output: true,
        ..CallbackGuestShape::VALID
    })
}

fn assemble_shaped_callback_guest(shape: CallbackGuestShape) -> Vec<u8> {
    assemble_callback_state_guest_with_bodies(
        i64_constant(OUTPUT_POINTER),
        vec![0, 0x41, 0, 0x0B],
        manifest_body(),
        state_import_body(),
        shape,
    )
}

pub fn malformed_manifest_guest() -> Vec<u8> {
    assemble_manifest_guest(output_body(AMNF_POINTER, AMNF.len() - 1))
}

pub fn repeated_undersized_manifest_guest() -> Vec<u8> {
    assemble_manifest_guest(repeated_undersized_manifest_body())
}

pub fn invalid_manifest_pointer_guest() -> Vec<u8> {
    assemble_callback_state_guest_with_bodies(
        i64_constant(65_500),
        vec![0, 0x41, 0, 0x0B],
        manifest_body(),
        state_import_body(),
        CallbackGuestShape::VALID,
    )
}

pub fn trapping_manifest_guest() -> Vec<u8> {
    assemble_manifest_guest(vec![0, 0x00, 0x0B])
}

pub fn infinite_manifest_guest() -> Vec<u8> {
    assemble_manifest_guest(vec![0, 0x03, 0x40, 0x0C, 0, 0x0B, 0x42, 0, 0x0B])
}

pub fn manifest_cleanup_failure_guest() -> Vec<u8> {
    assemble_callback_state_guest_with_bodies(
        i64_constant(OUTPUT_POINTER),
        vec![0, 0x41, 13, 0x0B],
        manifest_body(),
        state_import_body(),
        CallbackGuestShape::VALID,
    )
}

pub fn malformed_manifest_and_cleanup_failure_guest() -> Vec<u8> {
    assemble_callback_state_guest_with_bodies(
        i64_constant(OUTPUT_POINTER),
        vec![0, 0x41, 13, 0x0B],
        output_body(AMNF_POINTER, AMNF.len() - 1),
        state_import_body(),
        CallbackGuestShape::VALID,
    )
}

fn assemble_manifest_guest(manifest_body: Vec<u8>) -> Vec<u8> {
    assemble_callback_state_guest_with_bodies(
        i64_constant(OUTPUT_POINTER),
        vec![0, 0x41, 0, 0x0B],
        manifest_body,
        state_import_body(),
        CallbackGuestShape::VALID,
    )
}

pub fn unsupported_import_guest() -> Vec<u8> {
    let mut module = b"\0asm\x01\0\0\0".to_vec();
    push_section(&mut module, 1, &[1, 0x60, 0, 0]);

    let mut imports = vec![1];
    push_name(&mut imports, "environment");
    push_name(&mut imports, "unsupported");
    imports.extend_from_slice(&[0, 0]);
    push_section(&mut module, 2, &imports);
    module
}

pub fn incompatible_import_status_guest() -> Vec<u8> {
    assemble_callback_state_guest(OUTPUT_POINTER, 0, i64_constant(7_i64 << 32))
}

pub fn invalid_import_pointer_guest() -> Vec<u8> {
    assemble_callback_state_guest_with_bodies(
        cleanup_sensitive_allocation_body(),
        cleanup_marking_deallocation_body(0),
        manifest_body(),
        state_import_body(),
        CallbackGuestShape::VALID,
    )
}

pub fn trapping_import_guest() -> Vec<u8> {
    assemble_callback_state_guest(OUTPUT_POINTER, 0, vec![0, 0x00, 0x0B])
}

pub fn infinite_import_guest() -> Vec<u8> {
    assemble_callback_state_guest(
        OUTPUT_POINTER,
        0,
        vec![0, 0x03, 0x40, 0x0C, 0, 0x0B, 0x42, 0, 0x0B],
    )
}

pub fn import_cleanup_failure_guest() -> Vec<u8> {
    assemble_callback_state_guest(OUTPUT_POINTER, 13, state_import_body())
}

pub fn ignoring_import_guest() -> Vec<u8> {
    assemble_callback_state_guest(OUTPUT_POINTER, 0, vec![0, 0x42, 0, 0x0B])
}

pub fn candidate_migration_guest() -> Vec<u8> {
    assemble_shaped_callback_guest(CallbackGuestShape {
        migration_export: true,
        upgraded_state: true,
        ..CallbackGuestShape::VALID
    })
}

pub fn trapping_migration_guest() -> Vec<u8> {
    assemble_shaped_callback_guest(CallbackGuestShape {
        migration_export: true,
        upgraded_state: true,
        migration_trap: true,
        ..CallbackGuestShape::VALID
    })
}

pub fn unneeded_trapping_migration_guest() -> Vec<u8> {
    assemble_shaped_callback_guest(CallbackGuestShape {
        migration_export: true,
        migration_trap: true,
        ..CallbackGuestShape::VALID
    })
}

pub fn infinite_migration_guest() -> Vec<u8> {
    assemble_shaped_callback_guest(CallbackGuestShape {
        migration_export: true,
        upgraded_state: true,
        migration_infinite: true,
        ..CallbackGuestShape::VALID
    })
}

pub fn malformed_migration_guest() -> Vec<u8> {
    let mut module = candidate_migration_guest();
    replace_last(&mut module, b"ASTA", b"NOPE");
    module
}

pub fn substituted_migration_state_guest() -> Vec<u8> {
    let mut module = candidate_migration_guest();
    replace_last(&mut module, &[0x20; 16], &[0x21; 16]);
    module
}

pub fn import_and_cleanup_failure_guest() -> Vec<u8> {
    assemble_callback_state_guest_with_bodies(
        i64_constant(OUTPUT_POINTER),
        cleanup_marking_deallocation_body(13),
        manifest_body(),
        cleanup_sensitive_import_body(),
        CallbackGuestShape::VALID,
    )
}

fn assemble_callback_state_guest(
    allocation_pointer: i64,
    deallocation_status: u8,
    import_state_body: Vec<u8>,
) -> Vec<u8> {
    assemble_callback_state_guest_with_bodies(
        i64_constant(allocation_pointer),
        vec![0, 0x41, deallocation_status, 0x0B],
        manifest_body(),
        import_state_body,
        CallbackGuestShape::VALID,
    )
}

fn assemble_callback_state_guest_with_bodies(
    allocation_body: Vec<u8>,
    deallocation_body: Vec<u8>,
    manifest_body: Vec<u8>,
    import_state_body: Vec<u8>,
    shape: CallbackGuestShape,
) -> Vec<u8> {
    let mut module = b"\0asm\x01\0\0\0".to_vec();

    let mut types = vec![
        4 + u8::from(shape.start) + u8::from(shape.capability_import),
        0x60,
        0,
        1,
        0x7E,
        0x60,
        2,
        0x7F,
        0x7F,
        1,
        0x7E,
    ];
    types.extend_from_slice(&[0x60, 3, 0x7F, 0x7F, 0x7F, 1, 0x7F]);
    types.extend_from_slice(&[0x60, 4, 0x7F, 0x7F, 0x7F, 0x7F, 1, 0x7E]);
    if shape.start {
        types.extend_from_slice(&[0x60, 0, 0]);
    }
    let capability_type = 4 + u8::from(shape.start);
    if shape.capability_import {
        let parameter_count = if shape.wrong_capability_signature { 6 } else { 7 };
        types.extend_from_slice(&[0x60, parameter_count]);
        types.extend(std::iter::repeat_n(0x7F, parameter_count as usize));
        types.extend_from_slice(&[1, 0x7E]);
    }
    push_section(&mut module, 1, &types);

    if shape.capability_import {
        let mut imports = vec![1];
        push_name(&mut imports, "aimer");
        push_name(&mut imports, "capability_call");
        imports.extend_from_slice(&[0, capability_type]);
        push_section(&mut module, 2, &imports);
    }

    let function_count = 8
        + u8::from(shape.migration_export)
        + u8::from(shape.dispatch_async_event);
    let mut function_types = if shape.include_manifest {
        vec![function_count, 0, 1, 2, shape.manifest_type, 1, 3, 1, 1]
    } else {
        vec![function_count - 1, 0, 1, 2, 1, 3, 1, 1]
    };
    if shape.migration_export {
        function_types.push(3);
    }
    if shape.dispatch_async_event {
        function_types.push(3);
    }
    if shape.start {
        function_types[0] += 1;
        function_types.push(4);
    }
    push_section(&mut module, 3, &function_types);
    push_section(&mut module, 5, &[1, 0, 1]);

    let export_count = u8::from(shape.include_manifest)
        + u8::from(shape.unexpected_export)
        + u8::from(shape.migration_export)
        + u8::from(shape.dispatch_async_event)
        + 8;
    let mut exports = vec![export_count];
    push_export(&mut exports, "memory", 2, 0);
    let function_offset = u8::from(shape.capability_import);
    push_export(&mut exports, "aimer_abi_version", 0, function_offset);
    push_export(&mut exports, "aimer_alloc", 0, 1 + function_offset);
    push_export(&mut exports, "aimer_dealloc", 0, 2 + function_offset);
    if shape.include_manifest {
        push_export(&mut exports, "aimer_manifest", 0, 3 + function_offset);
    }
    let operation_offset = u8::from(shape.include_manifest);
    push_export(
        &mut exports,
        "aimer_build",
        0,
        3 + operation_offset + function_offset,
    );
    push_export(
        &mut exports,
        "aimer_dispatch_event",
        0,
        4 + operation_offset + function_offset,
    );
    push_export(
        &mut exports,
        "aimer_export_state",
        0,
        5 + operation_offset + function_offset,
    );
    push_export(
        &mut exports,
        "aimer_import_state",
        0,
        6 + operation_offset + function_offset,
    );
    if shape.migration_export {
        push_export(
            &mut exports,
            "aimer_migrate_state",
            0,
            7 + operation_offset + function_offset,
        );
    }
    if shape.dispatch_async_event {
        push_export(
            &mut exports,
            "aimer_dispatch_async_event",
            0,
            7 + operation_offset + function_offset + u8::from(shape.migration_export),
        );
    }
    if shape.unexpected_export {
        push_export(&mut exports, "unexpected", 0, 0);
    }
    push_section(&mut module, 7, &exports);
    if shape.start {
        push_section(
            &mut module,
            8,
            &[function_types[0] - 1 + function_offset],
        );
    }

    let mut code = vec![function_types[0]];
    push_body(&mut code, i64_constant(ABI_VERSION as i64));
    push_body(&mut code, allocation_body);
    push_body(&mut code, deallocation_body);
    if shape.include_manifest {
        push_body(&mut code, manifest_body);
    }
    push_body(
        &mut code,
        if shape.capability_build {
            capability_build_body(
                shape.strip_vector_prefix,
                shape.capability_request_length,
            )
        } else {
            build_body()
        },
    );
    push_body(
        &mut code,
        if shape.dispatch_widget_output {
            callback_widget_output_body(shape.partial_widget_output)
        } else if shape.mutate_widget_source {
            widget_source_mutating_dispatch_body()
        } else {
            dispatch_event_body()
        },
    );
    push_body(&mut code, state_export_body());
    push_body(&mut code, import_state_body);
    if shape.migration_export {
        let migration_body = if shape.migration_trap {
            vec![0, 0, 0x0B]
        } else if shape.migration_infinite {
            vec![0, 0x03, 0x40, 0x0C, 0, 0x0B, 0x42, 0, 0x0B]
        } else {
            migration_output_body(MIGRATED_ASTA_POINTER, ASTA_TEMPLATE.len())
        };
        push_body(&mut code, migration_body);
    }
    if shape.dispatch_async_event {
        push_body(
            &mut code,
            if shape.async_state_update {
                async_state_update_body()
            } else {
                i64_constant(0)
            },
        );
    }
    if shape.start {
        push_body(&mut code, vec![0, 0x0B]);
    }
    push_section(&mut module, 10, &code);

    let mut state_image = ASTA_TEMPLATE.to_vec();
    if shape.upgraded_state {
        state_image[24] = 8;
        state_image[80..84].copy_from_slice(&[3, 0, 0, 0]);
    }
    let mut data = vec![3 + u8::from(shape.migration_export)];
    push_data_segment(&mut data, AWIR_POINTER, AWIR);
    push_data_segment(&mut data, ASTA_POINTER, &state_image);
    push_data_segment(
        &mut data,
        AMNF_POINTER,
        if shape.capability_import {
            CAPABILITY_AMNF
        } else {
            AMNF
        },
    );
    if shape.migration_export {
        let mut migrated_state = state_image;
        *migrated_state.last_mut().unwrap() = 0xA5;
        push_data_segment(&mut data, MIGRATED_ASTA_POINTER, &migrated_state);
    }
    push_section(&mut module, 11, &data);

    module
}

pub fn repeated_undersized_guest() -> Vec<u8> {
    assemble_guest(ABI_VERSION, OUTPUT_POINTER, repeated_undersized_body())
}

pub fn invalid_pointer_guest() -> Vec<u8> {
    assemble_guest(ABI_VERSION, 65_500, build_body())
}

pub fn trapping_build_guest() -> Vec<u8> {
    assemble_guest(ABI_VERSION, OUTPUT_POINTER, vec![0, 0x00, 0x0B])
}

pub fn infinite_build_guest() -> Vec<u8> {
    assemble_guest(
        ABI_VERSION,
        OUTPUT_POINTER,
        vec![0, 0x03, 0x40, 0x0C, 0, 0x0B, 0x42, 0, 0x0B],
    )
}

fn assemble_guest(abi_version: u64, output_pointer: i64, build_body: Vec<u8>) -> Vec<u8> {
    let mut module = b"\0asm\x01\0\0\0".to_vec();

    let mut types = vec![3, 0x60, 0, 1, 0x7E, 0x60, 2, 0x7F, 0x7F, 1, 0x7E];
    types.extend_from_slice(&[0x60, 3, 0x7F, 0x7F, 0x7F, 1, 0x7F]);
    push_section(&mut module, 1, &types);

    push_section(&mut module, 3, &[4, 0, 1, 2, 1]);
    push_section(&mut module, 5, &[1, 0, 1]);

    let mut exports = vec![5];
    push_export(&mut exports, "memory", 2, 0);
    push_export(&mut exports, "aimer_abi_version", 0, 0);
    push_export(&mut exports, "aimer_alloc", 0, 1);
    push_export(&mut exports, "aimer_dealloc", 0, 2);
    push_export(&mut exports, "aimer_build", 0, 3);
    push_section(&mut module, 7, &exports);

    let mut code = vec![4];
    push_body(&mut code, i64_constant(abi_version as i64));
    push_body(&mut code, i64_constant(output_pointer));
    push_body(&mut code, vec![0, 0x41, 0, 0x0B]);
    push_body(&mut code, build_body);
    push_section(&mut module, 10, &code);

    let mut data = vec![1, 0, 0x41];
    push_signed_leb(&mut data, i64::from(AWIR_POINTER));
    data.push(0x0B);
    push_unsigned_leb(&mut data, AWIR.len() as u64);
    data.extend_from_slice(AWIR);
    push_section(&mut module, 11, &data);

    module
}

fn assemble_guest_with_diagnostic(abi_version: u64, diagnostic: Vec<u8>) -> Vec<u8> {
    assemble_guest_with_diagnostic_bytes(abi_version, diagnostic)
}

fn assemble_guest_with_diagnostic_bytes(abi_version: u64, diagnostic: Vec<u8>) -> Vec<u8> {
    const DIAGNOSTIC_POINTER: i32 = 12_288;

    let mut module = b"\0asm\x01\0\0\0".to_vec();
    let mut types = vec![4, 0x60, 0, 1, 0x7E, 0x60, 2, 0x7F, 0x7F, 1, 0x7E];
    types.extend_from_slice(&[0x60, 3, 0x7F, 0x7F, 0x7F, 1, 0x7F]);
    types.extend_from_slice(&[0x60, 2, 0x7F, 0x7F, 1, 0x7E]);
    push_section(&mut module, 1, &types);
    push_section(&mut module, 3, &[5, 0, 1, 2, 1, 1]);
    push_section(&mut module, 5, &[1, 0, 1]);

    let mut exports = vec![6];
    push_export(&mut exports, "memory", 2, 0);
    push_export(&mut exports, "aimer_abi_version", 0, 0);
    push_export(&mut exports, "aimer_alloc", 0, 1);
    push_export(&mut exports, "aimer_dealloc", 0, 2);
    push_export(&mut exports, "aimer_build", 0, 3);
    push_export(&mut exports, "aimer_diagnostic", 0, 4);
    push_section(&mut module, 7, &exports);

    let mut code = vec![5];
    push_body(&mut code, i64_constant(abi_version as i64));
    push_body(&mut code, i64_constant(OUTPUT_POINTER));
    push_body(&mut code, vec![0, 0x41, 0, 0x0B]);
    push_body(&mut code, i64_constant(12_i64 << 32));
    push_body(&mut code, output_body(DIAGNOSTIC_POINTER, diagnostic.len()));
    push_section(&mut module, 10, &code);

    let mut data = vec![1];
    push_data_segment(&mut data, DIAGNOSTIC_POINTER, &diagnostic);
    push_section(&mut module, 11, &data);
    module
}

fn build_body() -> Vec<u8> {
    let required = AWIR.len() as i64;
    let mut body = vec![0, 0x20, 1, 0x41];
    push_signed_leb(&mut body, required);
    body.extend_from_slice(&[0x49, 0x04, 0x7E, 0x42]);
    push_signed_leb(&mut body, (1_i64 << 32) | required);
    body.extend_from_slice(&[0x05, 0x20, 0, 0x41]);
    push_signed_leb(&mut body, i64::from(AWIR_POINTER));
    body.push(0x41);
    push_signed_leb(&mut body, required);
    body.extend_from_slice(&[0xFC, 0x0A, 0, 0, 0x42]);
    push_signed_leb(&mut body, required);
    body.extend_from_slice(&[0x0B, 0x0B]);
    body
}

fn repeated_undersized_body() -> Vec<u8> {
    let required = AWIR.len() as i64;
    let mut body = vec![0, 0x20, 1, 0x45, 0x04, 0x7E, 0x42];
    push_signed_leb(&mut body, (1_i64 << 32) | required);
    body.extend_from_slice(&[0x05, 0x42]);
    push_signed_leb(&mut body, (1_i64 << 32) | (required + 1));
    body.extend_from_slice(&[0x0B, 0x0B]);
    body
}

fn capability_build_body(strip_vector_prefix: bool, request_length: i32) -> Vec<u8> {
    let mut body = if strip_vector_prefix {
        vec![1, 1, 0x7E, 0x41]
    } else {
        vec![0, 0x41]
    };
    push_signed_leb(&mut body, i64::from(AMNF_POINTER + 64));
    body.extend_from_slice(&[0x41, 1, 0x41, 0, 0x41, 0, 0x41]);
    push_signed_leb(&mut body, i64::from(request_length));
    body.extend_from_slice(&[0x20, 0]);
    if strip_vector_prefix {
        body.extend_from_slice(&[0x41, 4, 0x6B]);
    }
    body.extend_from_slice(&[0x20, 1]);
    if strip_vector_prefix {
        body.extend_from_slice(&[0x41, 4, 0x6A]);
    }
    body.extend_from_slice(&[0x10, 0, 0x0B]);
    if strip_vector_prefix {
        body.pop();
        body.extend_from_slice(&[
            0x22, 2, 0x42, 32, 0x88, 0x42, 2, 0x54, 0x04, 0x7E, 0x20, 2, 0x42, 4, 0x7D,
            0x05, 0x20, 2, 0x0B, 0x0B,
        ]);
    }
    body
}

#[allow(dead_code)]
fn replace_last<const LENGTH: usize>(module: &mut [u8], old: &[u8; LENGTH], new: &[u8; LENGTH]) {
    let index = module
        .windows(LENGTH)
        .rposition(|window| window == old)
        .expect("fixture contract bytes");
    module[index..index + LENGTH].copy_from_slice(new);
}

fn dispatch_event_body() -> Vec<u8> {
    let mut body = vec![0, 0x41];
    push_signed_leb(&mut body, i64::from(ASTA_POINTER) + ASTA_TEMPLATE.len() as i64 - 1);
    body.extend_from_slice(&[0x20, 0, 0x41]);
    push_signed_leb(&mut body, 96);
    body.extend_from_slice(&[0x6A, 0x2D, 0, 0, 0x3A, 0, 0, 0x42, 0, 0x0B]);
    body
}

fn async_state_update_body() -> Vec<u8> {
    let mut body = vec![0, 0x41];
    push_signed_leb(&mut body, i64::from(AWIR_POINTER) + 8);
    body.extend_from_slice(&[0x20, 0, 0x41, 8, 0x6A, 0x29, 0, 0, 0x37, 0, 0, 0x41]);
    push_signed_leb(
        &mut body,
        i64::from(ASTA_POINTER) + ASTA_TEMPLATE.len() as i64 - 1,
    );
    body.extend_from_slice(&[0x41, 1, 0x3A, 0, 0]);
    let output = callback_widget_output_body(false);
    body.extend_from_slice(&output[1..]);
    body
}

fn widget_source_mutating_dispatch_body() -> Vec<u8> {
    let mut body = vec![0, 0x41];
    push_signed_leb(&mut body, i64::from(AWIR_POINTER));
    body.extend_from_slice(&[0x41, b'X', 0x3A, 0, 0, 0x42, 0, 0x0B]);
    body
}

fn callback_widget_output_body(partial: bool) -> Vec<u8> {
    let required = if partial { AWIR.len() - 1 } else { AWIR.len() } as i64;
    let mut body = vec![0, 0x20, 3, 0x41];
    push_signed_leb(&mut body, required);
    body.extend_from_slice(&[0x49, 0x04, 0x7E, 0x42]);
    push_signed_leb(&mut body, (1_i64 << 32) | required);
    body.extend_from_slice(&[0x05, 0x20, 2, 0x41]);
    push_signed_leb(&mut body, i64::from(AWIR_POINTER));
    body.push(0x41);
    push_signed_leb(&mut body, required);
    body.extend_from_slice(&[0xFC, 0x0A, 0, 0, 0x42]);
    push_signed_leb(&mut body, required);
    body.extend_from_slice(&[0x0B, 0x0B]);
    body
}

fn state_export_body() -> Vec<u8> {
    output_body(ASTA_POINTER, ASTA_TEMPLATE.len())
}

fn manifest_body() -> Vec<u8> {
    output_body(AMNF_POINTER, AMNF.len())
}

fn repeated_undersized_manifest_body() -> Vec<u8> {
    let required = AMNF.len() as i64;
    let mut body = vec![0, 0x20, 1, 0x45, 0x04, 0x7E, 0x42];
    push_signed_leb(&mut body, (1_i64 << 32) | required);
    body.extend_from_slice(&[0x05, 0x42]);
    push_signed_leb(&mut body, (1_i64 << 32) | (required + 1));
    body.extend_from_slice(&[0x0B, 0x0B]);
    body
}

fn state_import_body() -> Vec<u8> {
    let required = ASTA_TEMPLATE.len() as i64;
    let mut body = vec![0, 0x20, 1, 0x41];
    push_signed_leb(&mut body, required);
    body.extend_from_slice(&[0x47, 0x04, 0x7E, 0x42]);
    push_signed_leb(&mut body, 2_i64 << 32);
    body.extend_from_slice(&[0x05, 0x41]);
    push_signed_leb(&mut body, i64::from(ASTA_POINTER));
    body.extend_from_slice(&[0x20, 0, 0x20, 1, 0xFC, 0x0A, 0, 0, 0x42, 0, 0x0B, 0x0B]);
    body
}

fn migration_output_body(source_pointer: i32, output_len: usize) -> Vec<u8> {
    let required = output_len as i64;
    let mut body = vec![0, 0x20, 3, 0x41];
    push_signed_leb(&mut body, required);
    body.extend_from_slice(&[0x49, 0x04, 0x7E, 0x42]);
    push_signed_leb(&mut body, (1_i64 << 32) | required);
    body.extend_from_slice(&[0x05, 0x20, 2, 0x41]);
    push_signed_leb(&mut body, i64::from(source_pointer));
    body.push(0x41);
    push_signed_leb(&mut body, required);
    body.extend_from_slice(&[0xFC, 0x0A, 0, 0, 0x42]);
    push_signed_leb(&mut body, required);
    body.extend_from_slice(&[0x0B, 0x0B]);
    body
}

fn cleanup_sensitive_allocation_body() -> Vec<u8> {
    let mut body = vec![0, 0x41];
    push_signed_leb(&mut body, CLEANUP_MARKER_POINTER);
    body.extend_from_slice(&[0x2D, 0, 0, 0x45, 0x04, 0x7E, 0x42]);
    push_signed_leb(&mut body, 65_500);
    body.extend_from_slice(&[0x05, 0x42]);
    push_signed_leb(&mut body, OUTPUT_POINTER);
    body.extend_from_slice(&[0x0B, 0x0B]);
    body
}

fn cleanup_marking_deallocation_body(status: i64) -> Vec<u8> {
    let mut body = vec![0, 0x41];
    push_signed_leb(&mut body, CLEANUP_MARKER_POINTER);
    body.extend_from_slice(&[0x41, 1, 0x3A, 0, 0, 0x41]);
    push_signed_leb(&mut body, status);
    body.push(0x0B);
    body
}

fn cleanup_sensitive_import_body() -> Vec<u8> {
    let mut body = vec![0, 0x41];
    push_signed_leb(&mut body, CLEANUP_MARKER_POINTER);
    body.extend_from_slice(&[0x2D, 0, 0, 0x45, 0x04, 0x7E, 0x42]);
    push_signed_leb(&mut body, 7_i64 << 32);
    body.extend_from_slice(&[0x05, 0x42, 0, 0x0B, 0x0B]);
    body
}

fn output_body(source_pointer: i32, output_len: usize) -> Vec<u8> {
    let required = output_len as i64;
    let mut body = vec![0, 0x20, 1, 0x41];
    push_signed_leb(&mut body, required);
    body.extend_from_slice(&[0x49, 0x04, 0x7E, 0x42]);
    push_signed_leb(&mut body, (1_i64 << 32) | required);
    body.extend_from_slice(&[0x05, 0x20, 0, 0x41]);
    push_signed_leb(&mut body, i64::from(source_pointer));
    body.push(0x41);
    push_signed_leb(&mut body, required);
    body.extend_from_slice(&[0xFC, 0x0A, 0, 0, 0x42]);
    push_signed_leb(&mut body, required);
    body.extend_from_slice(&[0x0B, 0x0B]);
    body
}

fn i64_constant(value: i64) -> Vec<u8> {
    let mut body = vec![0, 0x42];
    push_signed_leb(&mut body, value);
    body.push(0x0B);
    body
}

fn push_export(output: &mut Vec<u8>, name: &str, kind: u8, index: u8) {
    push_name(output, name);
    output.extend_from_slice(&[kind, index]);
}

fn push_name(output: &mut Vec<u8>, name: &str) {
    push_unsigned_leb(output, name.len() as u64);
    output.extend_from_slice(name.as_bytes());
}

fn push_data_segment(output: &mut Vec<u8>, pointer: i32, bytes: &[u8]) {
    output.extend_from_slice(&[0, 0x41]);
    push_signed_leb(output, i64::from(pointer));
    output.push(0x0B);
    push_unsigned_leb(output, bytes.len() as u64);
    output.extend_from_slice(bytes);
}

fn push_body(output: &mut Vec<u8>, body: Vec<u8>) {
    push_unsigned_leb(output, body.len() as u64);
    output.extend_from_slice(&body);
}

fn push_section(module: &mut Vec<u8>, id: u8, payload: &[u8]) {
    module.push(id);
    push_unsigned_leb(module, payload.len() as u64);
    module.extend_from_slice(payload);
}

fn assemble_i32_guest(
    export_name: &str,
    memory: Option<&[u8]>,
    table: Option<&[u8]>,
    body: Vec<u8>,
) -> Vec<u8> {
    let mut module = b"\0asm\x01\0\0\0".to_vec();
    push_section(&mut module, 1, &[1, 0x60, 0, 1, 0x7F]);
    push_section(&mut module, 3, &[1, 0]);
    if let Some(table) = table {
        push_section(&mut module, 4, table);
    }
    if let Some(memory) = memory {
        push_section(&mut module, 5, memory);
    }
    let mut exports = vec![1];
    push_export(&mut exports, export_name, 0, 0);
    push_section(&mut module, 7, &exports);
    let mut code = vec![1];
    push_body(&mut code, body);
    push_section(&mut module, 10, &code);
    module
}

fn push_unsigned_leb(output: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            return;
        }
    }
}

fn push_signed_leb(output: &mut Vec<u8>, mut value: i64) {
    loop {
        let byte = (value as u8) & 0x7F;
        value >>= 7;
        let done = (value == 0 && byte & 0x40 == 0) || (value == -1 && byte & 0x40 != 0);
        output.push(if done { byte } else { byte | 0x80 });
        if done {
            return;
        }
    }
}
