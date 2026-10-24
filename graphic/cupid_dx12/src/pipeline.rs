use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aimer_utils::info;
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCOMPILE_OPTIMIZATION_LEVEL0, D3DCompile};
use windows::Win32::Graphics::Direct3D::{
    D3D_PRIMITIVE_TOPOLOGY, D3D_SHADER_MACRO, D3D11_PRIMITIVE_TOPOLOGY_LINELIST,
    D3D11_PRIMITIVE_TOPOLOGY_LINESTRIP, D3D11_PRIMITIVE_TOPOLOGY_POINTLIST,
    D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, ID3DBlob,
};
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::core::PCSTR;

use crate::backend::{
    BackendError, BindGroupLayoutEntry, BindingResource, BindingType, BlendFactor, BlendOperation,
    BufferBindingType, ColorTargetState, Face, FrontFace, PrimitiveTopology,
    RenderPipelineDescriptor, VertexFormat, VertexStepMode,
};

use super::resource::{Dx12Buffer, Dx12Sampler, Dx12TextureView};
use super::{Dx12Backend, Dx12TextureFormat};

#[derive(Clone)]
/// HLSL source retained for eager Direct3D shader compilation.
#[doc(hidden)]
pub struct Dx12ShaderModule {
    pub(super) source: Arc<str>,
    pub(super) label: String,
    pub(super) compiled: Arc<Mutex<HashMap<(String, Vec<u8>, u32), Arc<[u8]>>>>,
}

#[derive(Clone)]
/// Opaque bind-group layout for the native Direct3D 12 backend.
#[doc(hidden)]
pub struct Dx12BindGroupLayout {
    pub(super) entries: Vec<BindGroupLayoutEntry>,
}

#[derive(Clone)]
pub(super) enum BoundResource {
    Buffer {
        buffer: Dx12Buffer,
        offset: u64,
        size: u64,
    },
    Texture {
        view: Dx12TextureView,
    },
    Sampler {
        sampler: Dx12Sampler,
    },
}

#[derive(Clone)]
/// Opaque bind group retained by Cupid's Direct3D 12 backend.
#[doc(hidden)]
pub struct Dx12BindGroup {
    pub(super) entries: Vec<(u32, BoundResource)>,
}

#[derive(Clone, Copy)]
pub(super) enum RootBindingKind {
    Buffer(BufferBindingType),
    Texture,
    Sampler,
    StorageTexture(bool),
}

#[derive(Clone, Copy)]
pub(super) struct BindingPlan {
    pub(super) binding: u32,
    pub(super) root_index: u32,
    pub(super) kind: RootBindingKind,
    pub(super) dynamic_offset_index: Option<usize>,
}

#[derive(Clone)]
pub(super) struct GroupPlan {
    pub(super) bindings: Vec<BindingPlan>,
}

#[derive(Clone)]
/// Native root signature and binding map for a render pipeline.
#[doc(hidden)]
pub struct Dx12PipelineLayout {
    pub(super) root_signature: ID3D12RootSignature,
    pub(super) groups: Vec<GroupPlan>,
}

#[derive(Clone)]
/// Compiled Direct3D 12 graphics pipeline state used by Cupid.
#[doc(hidden)]
pub struct Dx12RenderPipeline {
    pub(super) state: ID3D12PipelineState,
    pub(super) layout: Dx12PipelineLayout,
    pub(super) topology: D3D_PRIMITIVE_TOPOLOGY,
    pub(super) vertex_strides: Vec<u32>,
}

impl Dx12Backend {
    pub(super) fn make_pipeline_layout(
        &self,
        layouts: &[&Dx12BindGroupLayout],
    ) -> Dx12PipelineLayout {
        let layout_started = Instant::now();
        let range_count = layouts.iter().map(|layout| layout.entries.len()).sum();
        let mut ranges: Vec<D3D12_DESCRIPTOR_RANGE> = Vec::with_capacity(range_count);
        let mut parameters: Vec<D3D12_ROOT_PARAMETER> = Vec::with_capacity(range_count);
        let mut groups = Vec::with_capacity(layouts.len());
        for (space, layout) in layouts.iter().enumerate() {
            let mut plans = Vec::with_capacity(layout.entries.len());
            let mut dynamic_index = 0usize;
            for entry in &layout.entries {
                assert!(
                    entry.count.is_none_or(|count| count.get() == 1),
                    "D3D12 bind group descriptor arrays are not supported yet"
                );
                let root_index = parameters.len() as u32;
                let dynamic_offset_index = match &entry.ty {
                    BindingType::Buffer {
                        has_dynamic_offset: true,
                        ..
                    } => {
                        let index = dynamic_index;
                        dynamic_index += 1;
                        Some(index)
                    }
                    _ => None,
                };
                let (kind, parameter) = match &entry.ty {
                    BindingType::Buffer { ty, .. } => {
                        let parameter_type = match *ty {
                            BufferBindingType::Uniform => D3D12_ROOT_PARAMETER_TYPE_CBV,
                            BufferBindingType::ReadOnlyStorage => D3D12_ROOT_PARAMETER_TYPE_SRV,
                            BufferBindingType::Storage => D3D12_ROOT_PARAMETER_TYPE_UAV,
                        };
                        let descriptor = D3D12_ROOT_DESCRIPTOR {
                            ShaderRegister: entry.binding,
                            RegisterSpace: space as u32,
                        };
                        (
                            RootBindingKind::Buffer(*ty),
                            D3D12_ROOT_PARAMETER {
                                ParameterType: parameter_type,
                                Anonymous: D3D12_ROOT_PARAMETER_0 {
                                    Descriptor: descriptor,
                                },
                                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
                            },
                        )
                    }
                    BindingType::Texture { .. } => {
                        let range_index = ranges.len();
                        ranges.push(D3D12_DESCRIPTOR_RANGE {
                            RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                            NumDescriptors: 1,
                            BaseShaderRegister: entry.binding,
                            RegisterSpace: space as u32,
                            OffsetInDescriptorsFromTableStart: D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND,
                        });
                        let table = D3D12_ROOT_DESCRIPTOR_TABLE {
                            NumDescriptorRanges: 1,
                            pDescriptorRanges: &ranges[range_index],
                        };
                        (
                            RootBindingKind::Texture,
                            D3D12_ROOT_PARAMETER {
                                ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                                Anonymous: D3D12_ROOT_PARAMETER_0 {
                                    DescriptorTable: table,
                                },
                                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
                            },
                        )
                    }
                    BindingType::Sampler(_) => {
                        let range_index = ranges.len();
                        ranges.push(D3D12_DESCRIPTOR_RANGE {
                            RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER,
                            NumDescriptors: 1,
                            BaseShaderRegister: entry.binding,
                            RegisterSpace: space as u32,
                            OffsetInDescriptorsFromTableStart: D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND,
                        });
                        let table = D3D12_ROOT_DESCRIPTOR_TABLE {
                            NumDescriptorRanges: 1,
                            pDescriptorRanges: &ranges[range_index],
                        };
                        (
                            RootBindingKind::Sampler,
                            D3D12_ROOT_PARAMETER {
                                ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                                Anonymous: D3D12_ROOT_PARAMETER_0 {
                                    DescriptorTable: table,
                                },
                                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
                            },
                        )
                    }
                    BindingType::StorageTexture { access, .. } => {
                        let read_only =
                            matches!(*access, crate::backend::StorageTextureAccess::ReadOnly);
                        let range_index = ranges.len();
                        ranges.push(D3D12_DESCRIPTOR_RANGE {
                            RangeType: if read_only {
                                D3D12_DESCRIPTOR_RANGE_TYPE_SRV
                            } else {
                                D3D12_DESCRIPTOR_RANGE_TYPE_UAV
                            },
                            NumDescriptors: 1,
                            BaseShaderRegister: entry.binding,
                            RegisterSpace: space as u32,
                            OffsetInDescriptorsFromTableStart: D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND,
                        });
                        let table = D3D12_ROOT_DESCRIPTOR_TABLE {
                            NumDescriptorRanges: 1,
                            pDescriptorRanges: &ranges[range_index],
                        };
                        (
                            RootBindingKind::StorageTexture(read_only),
                            D3D12_ROOT_PARAMETER {
                                ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                                Anonymous: D3D12_ROOT_PARAMETER_0 {
                                    DescriptorTable: table,
                                },
                                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
                            },
                        )
                    }
                };
                plans.push(BindingPlan {
                    binding: entry.binding,
                    root_index,
                    kind,
                    dynamic_offset_index,
                });
                parameters.push(parameter);
            }
            groups.push(GroupPlan { bindings: plans });
        }

        let desc = D3D12_ROOT_SIGNATURE_DESC {
            NumParameters: parameters.len() as u32,
            pParameters: parameters.as_ptr(),
            NumStaticSamplers: 0,
            pStaticSamplers: std::ptr::null(),
            Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
        };
        let mut serialized = None;
        let mut errors = None;
        unsafe {
            D3D12SerializeRootSignature(
                &desc,
                D3D_ROOT_SIGNATURE_VERSION_1,
                &mut serialized,
                Some(&mut errors),
            )
        }
        .unwrap_or_else(|error| {
            panic!(
                "serialize D3D12 root signature failed: {}",
                blob_text(errors.as_ref()).unwrap_or_else(|| error.to_string())
            )
        });
        let blob = serialized.expect("root signature serialization returned no blob");
        let bytes = unsafe {
            std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize())
        };
        let root_signature = unsafe {
            self.shared
                .device
                .CreateRootSignature::<ID3D12RootSignature>(0, bytes)
        }
        .expect("create D3D12 root signature");
        info!(
            "DX12 root-signature layout timing: {} bind groups, {} parameters, serialization and creation {:.2} ms",
            layouts.len(),
            parameters.len(),
            layout_started.elapsed().as_secs_f64() * 1000.0
        );
        Dx12PipelineLayout {
            root_signature,
            groups,
        }
    }

    pub(super) fn make_pipeline(
        &self,
        desc: &RenderPipelineDescriptor<Self>,
    ) -> Result<Dx12RenderPipeline, BackendError> {
        let pipeline_started = Instant::now();
        let fallback_layout;
        let layout = if let Some(layout) = desc.layout {
            layout.clone()
        } else {
            fallback_layout = self.make_pipeline_layout(&[]);
            fallback_layout
        };
        let vertex_started = Instant::now();
        let vs = compile_hlsl(&desc.vertex.module, desc.vertex.entry_point, b"vs_5_1\0").map_err(
            |message| BackendError {
                operation: "compile_vertex_shader",
                message: format!("{}: {message}", desc.vertex.module.label),
            },
        )?;
        let vertex_elapsed = vertex_started.elapsed();
        let (ps, pixel_elapsed) = if let Some(fragment) = &desc.fragment {
            let pixel_started = Instant::now();
            let shader = compile_hlsl(&fragment.module, fragment.entry_point, b"ps_5_1\0")
                .map_err(|message| BackendError {
                    operation: "compile_pixel_shader",
                    message: format!("{}: {message}", fragment.module.label),
                })?;
            (Some(shader), pixel_started.elapsed())
        } else {
            (None, Duration::ZERO)
        };

        let mut input_elements = Vec::new();
        let mut vertex_strides = Vec::with_capacity(desc.vertex.buffers.len());
        for (slot, layout) in desc.vertex.buffers.iter().enumerate() {
            if let Some(layout) = layout {
                vertex_strides.push(layout.array_stride as u32);
                for attribute in layout.attributes {
                    input_elements.push(D3D12_INPUT_ELEMENT_DESC {
                        SemanticName: PCSTR(b"TEXCOORD\0".as_ptr()),
                        SemanticIndex: attribute.shader_location,
                        Format: vertex_format(attribute.format),
                        InputSlot: slot as u32,
                        AlignedByteOffset: attribute.offset as u32,
                        InputSlotClass: if layout.step_mode == VertexStepMode::Instance {
                            D3D12_INPUT_CLASSIFICATION_PER_INSTANCE_DATA
                        } else {
                            D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA
                        },
                        InstanceDataStepRate: u32::from(
                            layout.step_mode == VertexStepMode::Instance,
                        ),
                    });
                }
            } else {
                vertex_strides.push(0);
            }
        }

        let mut pipeline_desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC::default();
        pipeline_desc.pRootSignature = ManuallyDrop::new(Some(layout.root_signature.clone()));
        pipeline_desc.VS = shader_bytecode(&vs);
        if let Some(ps) = &ps {
            pipeline_desc.PS = shader_bytecode(ps);
        }
        pipeline_desc.InputLayout = D3D12_INPUT_LAYOUT_DESC {
            pInputElementDescs: input_elements.as_ptr(),
            NumElements: input_elements.len() as u32,
        };
        pipeline_desc.BlendState = blend_desc(
            desc.fragment
                .as_ref()
                .and_then(|fragment| fragment.targets.first().and_then(Option::as_ref)),
        );
        pipeline_desc.SampleMask = u32::MAX;
        pipeline_desc.RasterizerState =
            rasterizer_desc(desc.primitive.front_face, desc.primitive.cull_mode);
        pipeline_desc.PrimitiveTopologyType = topology_type(desc.primitive.topology);
        pipeline_desc.SampleDesc = DXGI_SAMPLE_DESC {
            Count: desc.multisample.count.max(1),
            Quality: 0,
        };
        if let Some(fragment) = &desc.fragment {
            for (index, target) in fragment.targets.iter().enumerate() {
                if let Some(target) = target {
                    pipeline_desc.RTVFormats[index] = target.format.dxgi();
                    pipeline_desc.NumRenderTargets = index as u32 + 1;
                }
            }
        }
        if desc.depth_stencil.is_some() {
            return Err(BackendError {
                operation: "create_render_pipeline",
                message: "D3D12 depth-stencil pipeline state is not implemented yet".to_string(),
            });
        }
        let pso_started = Instant::now();
        let state_result = unsafe {
            self.shared
                .device
                .CreateGraphicsPipelineState::<ID3D12PipelineState>(&pipeline_desc)
        };
        let pso_elapsed = pso_started.elapsed();
        let state = state_result.map_err(|error| BackendError {
            operation: "create_render_pipeline",
            message: format!("{}: {error}", desc.label.as_deref().unwrap_or("pipeline")),
        });
        // SAFETY: the descriptor does not own this cloned COM reference after
        // CreateGraphicsPipelineState returns.
        unsafe { ManuallyDrop::drop(&mut pipeline_desc.pRootSignature) };
        let state = state?;
        let total_elapsed = pipeline_started.elapsed();
        let other_elapsed = total_elapsed
            .saturating_sub(vertex_elapsed)
            .saturating_sub(pixel_elapsed)
            .saturating_sub(pso_elapsed);
        info!(
            "DX12 pipeline timing: {} vertex HLSL {:.2} ms, pixel HLSL {:.2} ms, PSO creation {:.2} ms, other setup {:.2} ms, total {:.2} ms",
            desc.label.as_deref().unwrap_or("pipeline"),
            vertex_elapsed.as_secs_f64() * 1000.0,
            pixel_elapsed.as_secs_f64() * 1000.0,
            pso_elapsed.as_secs_f64() * 1000.0,
            other_elapsed.as_secs_f64() * 1000.0,
            total_elapsed.as_secs_f64() * 1000.0
        );
        Ok(Dx12RenderPipeline {
            state,
            layout,
            topology: primitive_topology(desc.primitive.topology),
            vertex_strides,
        })
    }
}

pub(super) fn compile_hlsl(
    module: &Dx12ShaderModule,
    entry_point: &str,
    target: &'static [u8],
) -> Result<Arc<[u8]>, String> {
    let entry = std::ffi::CString::new(entry_point).map_err(|error| error.to_string())?;
    let flags = if cfg!(debug_assertions) {
        // Keep development startup responsive; release builds retain FXC's
        // default optimization level for shader execution speed.
        D3DCOMPILE_OPTIMIZATION_LEVEL0
    } else {
        0
    };
    let key = (entry_point.to_string(), target.to_vec(), flags);
    if let Some(bytecode) = module
        .compiled
        .lock()
        .map_err(|_| "DX12 shader bytecode cache is poisoned".to_string())?
        .get(&key)
        .cloned()
    {
        return Ok(bytecode);
    }

    let mut code = None;
    let mut errors = None;
    let result = unsafe {
        D3DCompile(
            module.source.as_ptr().cast(),
            module.source.len(),
            PCSTR(b"cupid-hlsl\0".as_ptr()),
            None::<*const D3D_SHADER_MACRO>,
            None::<&windows::Win32::Graphics::Direct3D::ID3DInclude>,
            PCSTR(entry.as_ptr().cast()),
            PCSTR(target.as_ptr()),
            flags,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    if let Err(error) = result {
        return Err(blob_text(errors.as_ref()).unwrap_or_else(|| error.to_string()));
    }
    let code = code.ok_or_else(|| "D3DCompile returned no bytecode".to_string())?;
    let length = unsafe { code.GetBufferSize() };
    let pointer = unsafe { code.GetBufferPointer().cast::<u8>() };
    // SAFETY: the blob owns this buffer and stays alive until the bytes are copied.
    let bytecode = Arc::<[u8]>::from(unsafe { std::slice::from_raw_parts(pointer, length) });
    let mut cache = module
        .compiled
        .lock()
        .map_err(|_| "DX12 shader bytecode cache is poisoned".to_string())?;
    Ok(cache.entry(key).or_insert(bytecode).clone())
}

pub(super) fn blob_text(blob: Option<&ID3DBlob>) -> Option<String> {
    let blob = blob?;
    let length = unsafe { blob.GetBufferSize() };
    let pointer = unsafe { blob.GetBufferPointer().cast::<u8>() };
    let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };
    Some(
        String::from_utf8_lossy(bytes)
            .trim_end_matches('\0')
            .to_string(),
    )
}

fn shader_bytecode(bytes: &[u8]) -> D3D12_SHADER_BYTECODE {
    D3D12_SHADER_BYTECODE {
        pShaderBytecode: bytes.as_ptr().cast(),
        BytecodeLength: bytes.len(),
    }
}

fn vertex_format(format: VertexFormat) -> DXGI_FORMAT {
    match format {
        VertexFormat::Uint8x2 => DXGI_FORMAT_R8G8_UINT,
        VertexFormat::Uint8x4 => DXGI_FORMAT_R8G8B8A8_UINT,
        VertexFormat::Sint8x2 => DXGI_FORMAT_R8G8_SINT,
        VertexFormat::Sint8x4 => DXGI_FORMAT_R8G8B8A8_SINT,
        VertexFormat::Unorm8x2 => DXGI_FORMAT_R8G8_UNORM,
        VertexFormat::Unorm8x4 => DXGI_FORMAT_R8G8B8A8_UNORM,
        VertexFormat::Snorm8x2 => DXGI_FORMAT_R8G8_SNORM,
        VertexFormat::Snorm8x4 => DXGI_FORMAT_R8G8B8A8_SNORM,
        VertexFormat::Uint16x2 => DXGI_FORMAT_R16G16_UINT,
        VertexFormat::Uint16x4 => DXGI_FORMAT_R16G16B16A16_UINT,
        VertexFormat::Sint16x2 => DXGI_FORMAT_R16G16_SINT,
        VertexFormat::Sint16x4 => DXGI_FORMAT_R16G16B16A16_SINT,
        VertexFormat::Unorm16x2 => DXGI_FORMAT_R16G16_UNORM,
        VertexFormat::Unorm16x4 => DXGI_FORMAT_R16G16B16A16_UNORM,
        VertexFormat::Snorm16x2 => DXGI_FORMAT_R16G16_SNORM,
        VertexFormat::Snorm16x4 => DXGI_FORMAT_R16G16B16A16_SNORM,
        VertexFormat::Float16x2 => DXGI_FORMAT_R16G16_FLOAT,
        VertexFormat::Float16x4 => DXGI_FORMAT_R16G16B16A16_FLOAT,
        VertexFormat::Float32 => DXGI_FORMAT_R32_FLOAT,
        VertexFormat::Float32x2 => DXGI_FORMAT_R32G32_FLOAT,
        VertexFormat::Float32x3 => DXGI_FORMAT_R32G32B32_FLOAT,
        VertexFormat::Float32x4 => DXGI_FORMAT_R32G32B32A32_FLOAT,
        VertexFormat::Uint32 => DXGI_FORMAT_R32_UINT,
        VertexFormat::Uint32x2 => DXGI_FORMAT_R32G32_UINT,
        VertexFormat::Uint32x3 => DXGI_FORMAT_R32G32B32_UINT,
        VertexFormat::Uint32x4 => DXGI_FORMAT_R32G32B32A32_UINT,
        VertexFormat::Sint32 => DXGI_FORMAT_R32_SINT,
        VertexFormat::Sint32x2 => DXGI_FORMAT_R32G32_SINT,
        VertexFormat::Sint32x3 => DXGI_FORMAT_R32G32B32_SINT,
        VertexFormat::Sint32x4 => DXGI_FORMAT_R32G32B32A32_SINT,
    }
}

fn blend_desc(target: Option<&ColorTargetState<Dx12TextureFormat>>) -> D3D12_BLEND_DESC {
    let mut result = D3D12_BLEND_DESC::default();
    result.AlphaToCoverageEnable = false.into();
    result.IndependentBlendEnable = false.into();
    let attachment = &mut result.RenderTarget[0];
    attachment.RenderTargetWriteMask = target.map_or(0x0f, |target| {
        u8::from(target.write_mask.red)
            | (u8::from(target.write_mask.green) << 1)
            | (u8::from(target.write_mask.blue) << 2)
            | (u8::from(target.write_mask.alpha) << 3)
    });
    if let Some(blend) = target.and_then(|target| target.blend.as_ref()) {
        attachment.BlendEnable = true.into();
        attachment.SrcBlend = blend_factor(blend.color.src_factor);
        attachment.DestBlend = blend_factor(blend.color.dst_factor);
        attachment.BlendOp = blend_op(blend.color.operation);
        attachment.SrcBlendAlpha = blend_factor(blend.alpha.src_factor);
        attachment.DestBlendAlpha = blend_factor(blend.alpha.dst_factor);
        attachment.BlendOpAlpha = blend_op(blend.alpha.operation);
        attachment.LogicOpEnable = false.into();
    } else {
        attachment.BlendEnable = false.into();
    }
    result
}

fn blend_factor(factor: BlendFactor) -> D3D12_BLEND {
    match factor {
        BlendFactor::Zero => D3D12_BLEND_ZERO,
        BlendFactor::One => D3D12_BLEND_ONE,
        BlendFactor::Src => D3D12_BLEND_SRC_COLOR,
        BlendFactor::OneMinusSrc => D3D12_BLEND_INV_SRC_COLOR,
        BlendFactor::SrcAlpha => D3D12_BLEND_SRC_ALPHA,
        BlendFactor::OneMinusSrcAlpha => D3D12_BLEND_INV_SRC_ALPHA,
        BlendFactor::Dst => D3D12_BLEND_DEST_COLOR,
        BlendFactor::OneMinusDst => D3D12_BLEND_INV_DEST_COLOR,
        BlendFactor::DstAlpha => D3D12_BLEND_DEST_ALPHA,
        BlendFactor::OneMinusDstAlpha => D3D12_BLEND_INV_DEST_ALPHA,
        BlendFactor::SrcAlphaSaturated => D3D12_BLEND_SRC_ALPHA_SAT,
        BlendFactor::Constant => D3D12_BLEND_BLEND_FACTOR,
        BlendFactor::OneMinusConstant => D3D12_BLEND_INV_BLEND_FACTOR,
    }
}

fn blend_op(op: BlendOperation) -> D3D12_BLEND_OP {
    match op {
        BlendOperation::Add => D3D12_BLEND_OP_ADD,
        BlendOperation::Subtract => D3D12_BLEND_OP_SUBTRACT,
        BlendOperation::ReverseSubtract => D3D12_BLEND_OP_REV_SUBTRACT,
        BlendOperation::Min => D3D12_BLEND_OP_MIN,
        BlendOperation::Max => D3D12_BLEND_OP_MAX,
    }
}

fn rasterizer_desc(front_face: FrontFace, cull_mode: Option<Face>) -> D3D12_RASTERIZER_DESC {
    let mut desc = D3D12_RASTERIZER_DESC::default();
    desc.FillMode = D3D12_FILL_MODE_SOLID;
    desc.CullMode = match cull_mode {
        None => D3D12_CULL_MODE_NONE,
        Some(Face::Front) => D3D12_CULL_MODE_FRONT,
        Some(Face::Back) => D3D12_CULL_MODE_BACK,
    };
    desc.FrontCounterClockwise = (front_face == FrontFace::Ccw).into();
    desc.DepthClipEnable = true.into();
    desc
}

fn topology_type(topology: PrimitiveTopology) -> D3D12_PRIMITIVE_TOPOLOGY_TYPE {
    match topology {
        PrimitiveTopology::PointList => D3D12_PRIMITIVE_TOPOLOGY_TYPE_POINT,
        PrimitiveTopology::LineList | PrimitiveTopology::LineStrip => {
            D3D12_PRIMITIVE_TOPOLOGY_TYPE_LINE
        }
        PrimitiveTopology::TriangleList | PrimitiveTopology::TriangleStrip => {
            D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE
        }
    }
}

fn primitive_topology(topology: PrimitiveTopology) -> D3D_PRIMITIVE_TOPOLOGY {
    match topology {
        PrimitiveTopology::PointList => D3D11_PRIMITIVE_TOPOLOGY_POINTLIST,
        PrimitiveTopology::LineList => D3D11_PRIMITIVE_TOPOLOGY_LINELIST,
        PrimitiveTopology::LineStrip => D3D11_PRIMITIVE_TOPOLOGY_LINESTRIP,
        PrimitiveTopology::TriangleList => D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
        PrimitiveTopology::TriangleStrip => D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
    }
}

pub(super) fn bind_group_layout(entries: &[BindGroupLayoutEntry]) -> Dx12BindGroupLayout {
    Dx12BindGroupLayout {
        entries: entries.to_vec(),
    }
}

pub(super) fn bind_group(
    backend: &Dx12Backend,
    layout: &Dx12BindGroupLayout,
    entries: &[crate::backend::BindGroupEntry<Dx12Backend>],
) -> Dx12BindGroup {
    let mut resolved = Vec::with_capacity(entries.len());
    for entry in entries {
        let declared = layout
            .entries
            .iter()
            .find(|candidate| candidate.binding == entry.binding)
            .unwrap_or_else(|| panic!("binding {} is not in its D3D12 layout", entry.binding));
        let resource = match (&entry.resource, &declared.ty) {
            (BindingResource::Buffer(buffer), BindingType::Buffer { .. }) => {
                BoundResource::Buffer {
                    buffer: (**buffer).clone(),
                    offset: 0,
                    size: buffer.size,
                }
            }
            (BindingResource::BufferRange(buffer, offset, size), BindingType::Buffer { .. }) => {
                BoundResource::Buffer {
                    buffer: (**buffer).clone(),
                    offset: *offset,
                    size: *size,
                }
            }
            (BindingResource::TextureView(view), BindingType::Texture { .. })
            | (BindingResource::TextureView(view), BindingType::StorageTexture { .. }) => {
                BoundResource::Texture {
                    view: (**view).clone(),
                }
            }
            (BindingResource::Sampler(sampler), BindingType::Sampler(_)) => {
                BoundResource::Sampler {
                    sampler: (**sampler).clone(),
                }
            }
            _ => panic!(
                "bind group entry {} disagrees with its D3D12 layout",
                entry.binding
            ),
        };
        if let BoundResource::Texture { view } = &resource {
            let has_srv = view.srv.is_some();
            let has_uav = view.uav.is_some();
            match &declared.ty {
                BindingType::Texture { .. } => assert!(has_srv, "bound D3D12 texture has no SRV"),
                BindingType::StorageTexture { access, .. } => {
                    let needs_uav =
                        !matches!(access, crate::backend::StorageTextureAccess::ReadOnly);
                    assert!(
                        if needs_uav { has_uav } else { has_srv },
                        "bound D3D12 storage texture has no matching view"
                    );
                }
                _ => unreachable!("texture resource paired with a non-texture layout"),
            }
        }
        if let BoundResource::Sampler { .. } = &resource {
            let _ = &backend.shared.sampler_heap;
        }
        resolved.push((entry.binding, resource));
    }
    Dx12BindGroup { entries: resolved }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every HLSL source the DX12 backend compiles at startup.
    const SHADERS: &[(&str, &str)] = &[
        ("rect", include_str!("../../pipeline/shaders/dx12/rect.hlsl")),
        ("image", include_str!("../../pipeline/shaders/dx12/image.hlsl")),
        ("text", include_str!("../../pipeline/shaders/dx12/text.hlsl")),
        ("text_color", include_str!("../../pipeline/shaders/dx12/text_color.hlsl")),
        (
            "text_decoration",
            include_str!("../../pipeline/shaders/dx12/text_decoration.hlsl"),
        ),
        ("svg", include_str!("../../pipeline/shaders/dx12/svg.hlsl")),
        (
            "frame_composite",
            include_str!("../../pipeline/shaders/dx12/frame_composite.hlsl"),
        ),
        ("material", include_str!("../../pipeline/shaders/dx12/material.hlsl")),
        ("morph", include_str!("../../pipeline/shaders/dx12/morph.hlsl")),
    ];

    // FXC runs on the CPU, so this catches HLSL errors without a D3D12 device.
    #[test]
    fn every_builtin_shader_compiles_with_fxc() {
        let mut failures = Vec::new();
        for &(label, source) in SHADERS {
            let module = Dx12ShaderModule {
                source: Arc::from(source),
                label: label.to_string(),
                compiled: Arc::new(Mutex::new(HashMap::new())),
            };
            for (entry_point, target) in [("vs_main", b"vs_5_1\0"), ("fs_main", b"ps_5_1\0")] {
                if let Err(error) = compile_hlsl(&module, entry_point, target) {
                    failures.push(format!("{label}::{entry_point}: {error}"));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
