use std::collections::BTreeMap;
use std::fmt;

use naga::error::ShaderError;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use naga::{
    AddressSpace, Binding, BuiltIn, ImageClass, ImageDimension, Module, Scalar, ScalarKind,
    ShaderStage, SourceLocation, Span, TypeInner,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShaderContract {
    Screen2D,
    World3D,
}

const DEFAULT_IMPORTED_SCENE_SHADER_BODY: &str = include_str!("imported_scene_shader.wgsl");

const SCREEN2D_SHADER_PRELUDE: &str = r#"// VZGLYD shader contract v1: Screen2D
const VZGLYD_SHADER_CONTRACT_VERSION: u32 = 1u;

struct VzglydVertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) mode: f32,
};

struct VzglydVertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) mode: f32,
};

struct VzglydUniforms {
    time: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var t_diffuse: texture_2d<f32>;
@group(0) @binding(1) var t_font: texture_2d<f32>;
@group(0) @binding(2) var t_detail: texture_2d<f32>;
@group(0) @binding(3) var t_lookup: texture_2d<f32>;
@group(0) @binding(4) var s_diffuse: sampler;
@group(0) @binding(5) var s_font: sampler;
@group(0) @binding(6) var<uniform> u: VzglydUniforms;
"#;

const WORLD3D_SHADER_PRELUDE: &str = r#"// VZGLYD shader contract v1: World3D
const VZGLYD_SHADER_CONTRACT_VERSION: u32 = 1u;

struct VzglydVertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) mode: f32,
};

struct VzglydVertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) mode: f32,
};

struct VzglydUniforms {
    view_proj: mat4x4<f32>,
    cam_pos: vec3<f32>,
    time: f32,
    fog_color: vec4<f32>,
    fog_start: f32,
    fog_end: f32,
    clock_seconds: f32,
    _pad: f32,
    ambient_light: vec4<f32>,
    main_light_dir: vec4<f32>,
    main_light_color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: VzglydUniforms;
@group(0) @binding(1) var t_font: texture_2d<f32>;
@group(0) @binding(2) var t_noise: texture_2d<f32>;
@group(0) @binding(3) var t_material_a: texture_2d<f32>;
@group(0) @binding(4) var t_material_b: texture_2d<f32>;
@group(0) @binding(5) var s_clamp: sampler;
@group(0) @binding(6) var s_repeat: sampler;

fn vzglyd_ambient_light() -> vec3<f32> {
    return u.ambient_light.rgb;
}

fn vzglyd_main_light_dir() -> vec3<f32> {
    let dir = u.main_light_dir.xyz;
    let len_sq = dot(dir, dir);
    if len_sq <= 0.000001 {
        return vec3<f32>(0.0, 1.0, 0.0);
    }
    return normalize(dir);
}

fn vzglyd_main_light_rgb() -> vec3<f32> {
    return u.main_light_color.rgb;
}

fn vzglyd_main_light_strength() -> f32 {
    return max(max(u.main_light_color.r, u.main_light_color.g), u.main_light_color.b);
}

fn vzglyd_direct_light_scale() -> f32 {
    let ambient = vzglyd_ambient_light();
    return max(1.0 - max(max(ambient.r, ambient.g), ambient.b), 0.0);
}

fn vzglyd_main_light_screen_uv() -> vec2<f32> {
    let dir = vzglyd_main_light_dir();
    return clamp(
        vec2<f32>(0.5 + dir.x * 0.22, 0.5 - dir.y * 0.30),
        vec2<f32>(0.05, 0.05),
        vec2<f32>(0.95, 0.95),
    );
}
"#;

#[derive(Clone, Copy)]
struct InterfaceExpectation {
    semantic: &'static str,
    ty: &'static str,
}

#[derive(Clone, Copy)]
struct BindingExpectation {
    binding: u32,
    kind: BindingKind,
}

#[derive(Clone, Copy)]
struct ContractSpec {
    bindings: &'static [BindingExpectation],
    vertex_input: &'static [InterfaceExpectation],
    vertex_output: &'static [InterfaceExpectation],
    fragment_input: &'static [InterfaceExpectation],
    fragment_output: &'static [InterfaceExpectation],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BindingKind {
    UniformBuffer,
    Texture2DFloat,
    Sampler,
}

impl fmt::Display for BindingKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindingKind::UniformBuffer => write!(f, "a uniform buffer"),
            BindingKind::Texture2DFloat => write!(f, "a sampled `texture_2d<f32>`"),
            BindingKind::Sampler => write!(f, "a filtering sampler"),
        }
    }
}

const SCREEN2D_BINDINGS: [BindingExpectation; 7] = [
    BindingExpectation {
        binding: 0,
        kind: BindingKind::Texture2DFloat,
    },
    BindingExpectation {
        binding: 1,
        kind: BindingKind::Texture2DFloat,
    },
    BindingExpectation {
        binding: 2,
        kind: BindingKind::Texture2DFloat,
    },
    BindingExpectation {
        binding: 3,
        kind: BindingKind::Texture2DFloat,
    },
    BindingExpectation {
        binding: 4,
        kind: BindingKind::Sampler,
    },
    BindingExpectation {
        binding: 5,
        kind: BindingKind::Sampler,
    },
    BindingExpectation {
        binding: 6,
        kind: BindingKind::UniformBuffer,
    },
];

const WORLD3D_BINDINGS: [BindingExpectation; 7] = [
    BindingExpectation {
        binding: 0,
        kind: BindingKind::UniformBuffer,
    },
    BindingExpectation {
        binding: 1,
        kind: BindingKind::Texture2DFloat,
    },
    BindingExpectation {
        binding: 2,
        kind: BindingKind::Texture2DFloat,
    },
    BindingExpectation {
        binding: 3,
        kind: BindingKind::Texture2DFloat,
    },
    BindingExpectation {
        binding: 4,
        kind: BindingKind::Texture2DFloat,
    },
    BindingExpectation {
        binding: 5,
        kind: BindingKind::Sampler,
    },
    BindingExpectation {
        binding: 6,
        kind: BindingKind::Sampler,
    },
];

const SCREEN2D_VERTEX_INPUT: [InterfaceExpectation; 4] = [
    InterfaceExpectation {
        semantic: "@location(0)",
        ty: "vec3<f32>",
    },
    InterfaceExpectation {
        semantic: "@location(1)",
        ty: "vec2<f32>",
    },
    InterfaceExpectation {
        semantic: "@location(2)",
        ty: "vec4<f32>",
    },
    InterfaceExpectation {
        semantic: "@location(3)",
        ty: "f32",
    },
];

const SCREEN2D_VERTEX_OUTPUT: [InterfaceExpectation; 4] = [
    InterfaceExpectation {
        semantic: "@builtin(position)",
        ty: "vec4<f32>",
    },
    InterfaceExpectation {
        semantic: "@location(0)",
        ty: "vec2<f32>",
    },
    InterfaceExpectation {
        semantic: "@location(1)",
        ty: "vec4<f32>",
    },
    InterfaceExpectation {
        semantic: "@location(2)",
        ty: "f32",
    },
];

const WORLD3D_VERTEX_INPUT: [InterfaceExpectation; 4] = [
    InterfaceExpectation {
        semantic: "@location(0)",
        ty: "vec3<f32>",
    },
    InterfaceExpectation {
        semantic: "@location(1)",
        ty: "vec3<f32>",
    },
    InterfaceExpectation {
        semantic: "@location(2)",
        ty: "vec4<f32>",
    },
    InterfaceExpectation {
        semantic: "@location(3)",
        ty: "f32",
    },
];

const WORLD3D_VERTEX_OUTPUT: [InterfaceExpectation; 5] = [
    InterfaceExpectation {
        semantic: "@builtin(position)",
        ty: "vec4<f32>",
    },
    InterfaceExpectation {
        semantic: "@location(0)",
        ty: "vec3<f32>",
    },
    InterfaceExpectation {
        semantic: "@location(1)",
        ty: "vec3<f32>",
    },
    InterfaceExpectation {
        semantic: "@location(2)",
        ty: "vec4<f32>",
    },
    InterfaceExpectation {
        semantic: "@location(3)",
        ty: "f32",
    },
];

const FRAGMENT_OUTPUT: [InterfaceExpectation; 1] = [InterfaceExpectation {
    semantic: "@location(0)",
    ty: "vec4<f32>",
}];

const SCREEN2D_SPEC: ContractSpec = ContractSpec {
    bindings: &SCREEN2D_BINDINGS,
    vertex_input: &SCREEN2D_VERTEX_INPUT,
    vertex_output: &SCREEN2D_VERTEX_OUTPUT,
    fragment_input: &SCREEN2D_VERTEX_OUTPUT,
    fragment_output: &FRAGMENT_OUTPUT,
};

const WORLD3D_SPEC: ContractSpec = ContractSpec {
    bindings: &WORLD3D_BINDINGS,
    vertex_input: &WORLD3D_VERTEX_INPUT,
    vertex_output: &WORLD3D_VERTEX_OUTPUT,
    fragment_input: &WORLD3D_VERTEX_OUTPUT,
    fragment_output: &FRAGMENT_OUTPUT,
};

impl ShaderContract {
    fn spec(self) -> &'static ContractSpec {
        match self {
            ShaderContract::Screen2D => &SCREEN2D_SPEC,
            ShaderContract::World3D => &WORLD3D_SPEC,
        }
    }
}

pub fn shader_prelude(contract: ShaderContract) -> &'static str {
    match contract {
        ShaderContract::Screen2D => SCREEN2D_SHADER_PRELUDE,
        ShaderContract::World3D => WORLD3D_SHADER_PRELUDE,
    }
}

pub fn assembled_slide_shader_source(contract: ShaderContract, shader_body: &str) -> String {
    format!("{}\n{}", shader_prelude(contract), shader_body)
}

pub fn default_imported_scene_shader_source() -> Result<String, ShaderValidationError> {
    validate_slide_shader_body(
        "imported_scene_shader.wgsl",
        DEFAULT_IMPORTED_SCENE_SHADER_BODY,
        ShaderContract::World3D,
        "vs_main",
        "fs_main",
    )
}

#[derive(Debug)]
pub struct ShaderValidationError {
    summary: String,
    diagnostic: String,
    #[cfg(test)]
    location: Option<SourceLocation>,
}

impl ShaderValidationError {
    #[cfg(test)]
    pub fn summary(&self) -> &str {
        &self.summary
    }

    pub fn diagnostic(&self) -> &str {
        &self.diagnostic
    }

    #[cfg(test)]
    pub fn location(&self) -> Option<SourceLocation> {
        self.location
    }

    fn from_parse(label: &str, source: &str, error: naga::front::wgsl::ParseError) -> Self {
        let summary = error.to_string();
        let diagnostic = error.emit_to_string_with_path(source, label);
        #[cfg(test)]
        let location = error.location(source);
        Self {
            summary,
            diagnostic,
            #[cfg(test)]
            location,
        }
    }

    fn from_naga_validation(
        label: &str,
        source: &str,
        error: naga::WithSpan<naga::valid::ValidationError>,
    ) -> Self {
        let summary = error.to_string();
        #[cfg(test)]
        let location = error.spans().next().map(|(span, _)| span.location(source));
        let diagnostic = ShaderError {
            source: source.to_string(),
            label: Some(label.to_string()),
            inner: Box::new(error),
        }
        .to_string();
        Self {
            summary,
            diagnostic,
            #[cfg(test)]
            location,
        }
    }

    fn custom(label: &str, source: &str, summary: impl Into<String>, span: Option<Span>) -> Self {
        let summary = summary.into();
        let location = span
            .filter(|span| span.is_defined())
            .map(|span| span.location(source));
        let diagnostic = render_custom_diagnostic(label, source, &summary, location);
        Self {
            summary,
            diagnostic,
            #[cfg(test)]
            location,
        }
    }
}

impl fmt::Display for ShaderValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.summary)
    }
}

impl std::error::Error for ShaderValidationError {}

pub fn validate_shader_source(
    label: &str,
    source: &str,
    contract: ShaderContract,
    vs_entry: &str,
    fs_entry: &str,
) -> Result<(), ShaderValidationError> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|error| ShaderValidationError::from_parse(label, source, error))?;

    let mut validator = Validator::new(ValidationFlags::all(), Capabilities::all());
    validator
        .validate(&module)
        .map_err(|error| ShaderValidationError::from_naga_validation(label, source, error))?;

    reject_unsupported_features(label, source, &module)?;
    validate_bindings(label, source, &module, contract.spec())?;
    validate_entry_point(
        label,
        source,
        &module,
        ShaderStage::Vertex,
        vs_entry,
        contract.spec().vertex_input,
        contract.spec().vertex_output,
    )?;
    validate_entry_point(
        label,
        source,
        &module,
        ShaderStage::Fragment,
        fs_entry,
        contract.spec().fragment_input,
        contract.spec().fragment_output,
    )?;
    Ok(())
}

pub fn validate_slide_shader_body(
    label: &str,
    shader_body: &str,
    contract: ShaderContract,
    vs_entry: &str,
    fs_entry: &str,
) -> Result<String, ShaderValidationError> {
    reject_prelude_binding_conflicts(label, shader_body, contract)?;
    let shader_source = assembled_slide_shader_source(contract, shader_body);
    validate_shader_source(label, &shader_source, contract, vs_entry, fs_entry)?;
    Ok(shader_source)
}

fn reject_unsupported_features(
    label: &str,
    source: &str,
    module: &Module,
) -> Result<(), ShaderValidationError> {
    if let Some(entry_point) = module
        .entry_points
        .iter()
        .find(|entry_point| entry_point.stage == ShaderStage::Compute)
    {
        return Err(ShaderValidationError::custom(
            label,
            source,
            format!(
                "compute entry point '{}' is not supported in slide shaders",
                entry_point.name
            ),
            find_named_function_span(source, &entry_point.name)
                .or_else(|| find_token_span(source, "@compute")),
        ));
    }

    for (handle, global) in module.global_variables.iter() {
        if matches!(
            global.space,
            AddressSpace::Storage { .. } | AddressSpace::PushConstant
        ) {
            let feature = match global.space {
                AddressSpace::Storage { .. } => "storage buffers",
                AddressSpace::PushConstant => "push constants",
                _ => unreachable!(),
            };
            return Err(ShaderValidationError::custom(
                label,
                source,
                format!("{feature} are not supported in slide shaders"),
                Some(module.global_variables.get_span(handle)),
            ));
        }
    }

    Ok(())
}

fn reject_prelude_binding_conflicts(
    label: &str,
    source: &str,
    contract: ShaderContract,
) -> Result<(), ShaderValidationError> {
    for binding in contract.spec().bindings {
        if let Some(span) = find_group_binding_span(source, 0, binding.binding) {
            return Err(ShaderValidationError::custom(
                label,
                source,
                format!(
                    "binding @group(0) @binding({}) is reserved by the VZGLYD shader prelude",
                    binding.binding
                ),
                Some(span),
            ));
        }
    }

    Ok(())
}

fn validate_bindings(
    label: &str,
    source: &str,
    module: &Module,
    contract: &ContractSpec,
) -> Result<(), ShaderValidationError> {
    let expected: BTreeMap<u32, BindingKind> = contract
        .bindings
        .iter()
        .map(|binding| (binding.binding, binding.kind))
        .collect();

    for (handle, global) in module.global_variables.iter() {
        let Some(resource_binding) = &global.binding else {
            continue;
        };
        let span = Some(module.global_variables.get_span(handle));
        if resource_binding.group != 0 {
            return Err(ShaderValidationError::custom(
                label,
                source,
                format!(
                    "binding @group({}) @binding({}) is unsupported; slide shaders may only use bind group 0",
                    resource_binding.group, resource_binding.binding
                ),
                span,
            ));
        }

        let actual_kind = classify_binding_kind(module, global)
            .map_err(|message| ShaderValidationError::custom(label, source, message, span))?;

        match expected.get(&resource_binding.binding) {
            Some(expected_kind) if *expected_kind == actual_kind => {}
            Some(expected_kind) => {
                return Err(ShaderValidationError::custom(
                    label,
                    source,
                    format!(
                        "binding @group(0) @binding({}) must be {expected_kind}, found {actual_kind}",
                        resource_binding.binding
                    ),
                    span,
                ));
            }
            None => {
                return Err(ShaderValidationError::custom(
                    label,
                    source,
                    format!(
                        "binding @group(0) @binding({}) is not part of the engine contract",
                        resource_binding.binding
                    ),
                    span,
                ));
            }
        }
    }

    Ok(())
}

fn validate_entry_point(
    label: &str,
    source: &str,
    module: &Module,
    stage: ShaderStage,
    expected_name: &str,
    expected_input: &[InterfaceExpectation],
    expected_output: &[InterfaceExpectation],
) -> Result<(), ShaderValidationError> {
    let matching_entry = module
        .entry_points
        .iter()
        .find(|entry_point| entry_point.stage == stage && entry_point.name == expected_name);

    let entry_point = if let Some(entry_point) = matching_entry {
        entry_point
    } else {
        let available_names: Vec<&str> = module
            .entry_points
            .iter()
            .filter(|entry_point| entry_point.stage == stage)
            .map(|entry_point| entry_point.name.as_str())
            .collect();
        let summary = if available_names.is_empty() {
            format!(
                "shader is missing the required {} entry point '{expected_name}'",
                stage_name(stage)
            )
        } else {
            format!(
                "{} entry point must be named '{expected_name}', found {}",
                stage_name(stage),
                available_names
                    .iter()
                    .map(|name| format!("'{name}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        return Err(ShaderValidationError::custom(
            label,
            source,
            summary,
            find_token_span(source, stage_attribute(stage)),
        ));
    };

    let entry_span = find_named_function_span(source, &entry_point.name)
        .or_else(|| find_token_span(source, stage_attribute(stage)));
    let actual_input = collect_argument_interface(module, &entry_point.function.arguments)
        .map_err(|message| ShaderValidationError::custom(label, source, message, entry_span))?;
    let actual_output = collect_result_interface(module, entry_point.function.result.as_ref())
        .map_err(|message| ShaderValidationError::custom(label, source, message, entry_span))?;

    compare_interface(
        label,
        source,
        stage,
        "input",
        expected_name,
        expected_input,
        &actual_input,
        entry_span,
    )?;
    compare_interface(
        label,
        source,
        stage,
        "output",
        expected_name,
        expected_output,
        &actual_output,
        entry_span,
    )?;
    Ok(())
}

fn compare_interface(
    label: &str,
    source: &str,
    stage: ShaderStage,
    direction: &str,
    entry_name: &str,
    expected: &[InterfaceExpectation],
    actual: &BTreeMap<String, String>,
    span: Option<Span>,
) -> Result<(), ShaderValidationError> {
    let expected_map: BTreeMap<String, String> = expected
        .iter()
        .map(|item| (item.semantic.to_string(), item.ty.to_string()))
        .collect();

    if expected_map == *actual {
        return Ok(());
    }

    Err(ShaderValidationError::custom(
        label,
        source,
        format!(
            "{} entry point '{}' has the wrong {direction} interface; expected [{}], found [{}]",
            stage_name(stage),
            entry_name,
            format_interface_map(&expected_map),
            format_interface_map(actual)
        ),
        span,
    ))
}

fn collect_argument_interface(
    module: &Module,
    arguments: &[naga::FunctionArgument],
) -> Result<BTreeMap<String, String>, String> {
    let mut fields = BTreeMap::new();
    for argument in arguments {
        collect_interface_fields(module, argument.ty, argument.binding.as_ref(), &mut fields)?;
    }
    Ok(fields)
}

fn collect_result_interface(
    module: &Module,
    result: Option<&naga::FunctionResult>,
) -> Result<BTreeMap<String, String>, String> {
    let mut fields = BTreeMap::new();
    let Some(result) = result else {
        return Ok(fields);
    };
    collect_interface_fields(module, result.ty, result.binding.as_ref(), &mut fields)?;
    Ok(fields)
}

fn collect_interface_fields(
    module: &Module,
    ty: naga::Handle<naga::Type>,
    binding: Option<&Binding>,
    fields: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    if let Some(binding) = binding {
        insert_interface_field(
            fields,
            describe_interface_binding(binding),
            describe_type(module, ty),
        )
    } else {
        match &module.types[ty].inner {
            TypeInner::Struct { members, .. } => {
                for member in members {
                    let Some(binding) = member.binding.as_ref() else {
                        return Err(format!(
                            "IO struct member '{}' is missing a binding",
                            member.name.as_deref().unwrap_or("<unnamed>")
                        ));
                    };
                    insert_interface_field(
                        fields,
                        describe_interface_binding(binding),
                        describe_type(module, member.ty),
                    )?;
                }
                Ok(())
            }
            other => Err(format!(
                "entry point interface must be a bound value or IO struct, found {}",
                describe_type_inner(other)
            )),
        }
    }
}

fn insert_interface_field(
    fields: &mut BTreeMap<String, String>,
    semantic: String,
    ty: String,
) -> Result<(), String> {
    if fields.insert(semantic.clone(), ty.clone()).is_some() {
        return Err(format!(
            "entry point interface defines '{semantic}' more than once"
        ));
    }
    Ok(())
}

fn classify_binding_kind(
    module: &Module,
    global: &naga::GlobalVariable,
) -> Result<BindingKind, String> {
    match global.space {
        AddressSpace::Uniform => Ok(BindingKind::UniformBuffer),
        AddressSpace::Handle => match &module.types[global.ty].inner {
            TypeInner::Image {
                dim: ImageDimension::D2,
                arrayed: false,
                class:
                    ImageClass::Sampled {
                        kind: ScalarKind::Float,
                        multi: false,
                    },
            } => Ok(BindingKind::Texture2DFloat),
            TypeInner::Sampler { comparison: false } => Ok(BindingKind::Sampler),
            TypeInner::Image { class, .. } => Err(format!(
                "resource '{}' uses unsupported image type {}",
                global.name.as_deref().unwrap_or("<unnamed>"),
                describe_image_class(class)
            )),
            TypeInner::Sampler { comparison: true } => Err(format!(
                "resource '{}' uses an unsupported comparison sampler",
                global.name.as_deref().unwrap_or("<unnamed>")
            )),
            TypeInner::BindingArray { .. } => Err(format!(
                "resource '{}' uses an unsupported binding array",
                global.name.as_deref().unwrap_or("<unnamed>")
            )),
            other => Err(format!(
                "resource '{}' has unsupported bindable type {}",
                global.name.as_deref().unwrap_or("<unnamed>"),
                describe_type_inner(other)
            )),
        },
        AddressSpace::Storage { .. } => {
            Err("storage buffers are not supported in slide shaders".into())
        }
        AddressSpace::PushConstant => {
            Err("push constants are not supported in slide shaders".into())
        }
        other => Err(format!(
            "resource '{}' uses unsupported address space {other:?}",
            global.name.as_deref().unwrap_or("<unnamed>")
        )),
    }
}

fn describe_interface_binding(binding: &Binding) -> String {
    match binding {
        Binding::BuiltIn(BuiltIn::Position { .. }) => "@builtin(position)".to_string(),
        Binding::BuiltIn(other) => format!("@builtin({other:?})"),
        Binding::Location { location, .. } => format!("@location({location})"),
    }
}

fn describe_type(module: &Module, ty: naga::Handle<naga::Type>) -> String {
    describe_type_inner(&module.types[ty].inner)
}

fn describe_type_inner(inner: &TypeInner) -> String {
    match inner {
        TypeInner::Scalar(scalar) => describe_scalar(*scalar),
        TypeInner::Vector { size, scalar } => {
            format!("vec{}<{}>", *size as u8, describe_scalar(*scalar))
        }
        TypeInner::Matrix {
            columns,
            rows,
            scalar,
        } => format!(
            "mat{}x{}<{}>",
            *columns as u8,
            *rows as u8,
            describe_scalar(*scalar)
        ),
        TypeInner::Struct { .. } => "struct".to_string(),
        TypeInner::Image {
            dim,
            arrayed,
            class,
        } => match class {
            ImageClass::Sampled { kind, .. } => {
                format!(
                    "texture_{}<{}>",
                    describe_image_dimension(*dim, *arrayed),
                    kind_name(*kind)
                )
            }
            ImageClass::Depth { .. } => {
                format!("texture_depth_{}", describe_image_dimension(*dim, *arrayed))
            }
            ImageClass::Storage { format, .. } => format!(
                "texture_storage_{}<{format:?}>",
                describe_image_dimension(*dim, *arrayed)
            ),
        },
        TypeInner::Sampler { comparison } => {
            if *comparison {
                "sampler_comparison".to_string()
            } else {
                "sampler".to_string()
            }
        }
        TypeInner::Array { base, size, .. } => format!("array<{base:?}, {size:?}>"),
        TypeInner::Atomic(scalar) => format!("atomic<{}>", describe_scalar(*scalar)),
        TypeInner::Pointer { .. } => "ptr".to_string(),
        TypeInner::ValuePointer { .. } => "value_ptr".to_string(),
        TypeInner::BindingArray { .. } => "binding_array".to_string(),
        TypeInner::AccelerationStructure { .. } => "acceleration_structure".to_string(),
        TypeInner::RayQuery { .. } => "ray_query".to_string(),
    }
}

fn describe_scalar(scalar: Scalar) -> String {
    format!("{}{}", kind_name(scalar.kind), scalar_width_suffix(scalar))
}

fn kind_name(kind: ScalarKind) -> &'static str {
    match kind {
        ScalarKind::Sint => "i",
        ScalarKind::Uint => "u",
        ScalarKind::Float => "f",
        ScalarKind::Bool => "bool",
        ScalarKind::AbstractInt => "abstract-int",
        ScalarKind::AbstractFloat => "abstract-float",
    }
}

fn scalar_width_suffix(scalar: Scalar) -> &'static str {
    match (scalar.kind, scalar.width) {
        (ScalarKind::Bool, _) => "",
        (_, 4) => "32",
        (_, 8) => "64",
        (_, 2) => "16",
        _ => "",
    }
}

fn describe_image_class(class: &ImageClass) -> String {
    match class {
        ImageClass::Sampled { kind, multi } => {
            if *multi {
                format!("a multisampled sampled texture of {}", kind_name(*kind))
            } else {
                format!("a sampled texture of {}", kind_name(*kind))
            }
        }
        ImageClass::Depth { multi } => {
            if *multi {
                "a multisampled depth texture".to_string()
            } else {
                "a depth texture".to_string()
            }
        }
        ImageClass::Storage { format, .. } => format!("a storage texture with format {format:?}"),
    }
}

fn describe_image_dimension(dim: ImageDimension, arrayed: bool) -> &'static str {
    match (dim, arrayed) {
        (ImageDimension::D1, false) => "1d",
        (ImageDimension::D2, false) => "2d",
        (ImageDimension::D2, true) => "2d_array",
        (ImageDimension::D3, false) => "3d",
        (ImageDimension::Cube, false) => "cube",
        (ImageDimension::Cube, true) => "cube_array",
        (ImageDimension::D1, true) | (ImageDimension::D3, true) => "unsupported",
    }
}

fn format_interface_map(fields: &BTreeMap<String, String>) -> String {
    fields
        .iter()
        .map(|(semantic, ty)| format!("{semantic}: {ty}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_custom_diagnostic(
    label: &str,
    source: &str,
    summary: &str,
    location: Option<SourceLocation>,
) -> String {
    match location {
        Some(location) => {
            let line = source
                .lines()
                .nth((location.line_number.saturating_sub(1)) as usize)
                .unwrap_or("");
            let caret_padding = " ".repeat(location.line_position.saturating_sub(1) as usize);
            format!(
                "{label}:{}:{}: error: {summary}\n{line}\n{caret_padding}^\n",
                location.line_number, location.line_position
            )
        }
        None => format!("{label}: error: {summary}\n"),
    }
}

fn find_named_function_span(source: &str, name: &str) -> Option<Span> {
    let needle = format!("fn {name}");
    source.find(&needle).map(|offset| {
        let start = offset + 3;
        Span::new(start as u32, (start + name.len()) as u32)
    })
}

fn find_token_span(source: &str, token: &str) -> Option<Span> {
    source
        .find(token)
        .map(|offset| Span::new(offset as u32, (offset + token.len()) as u32))
}

fn find_group_binding_span(source: &str, group: u32, binding: u32) -> Option<Span> {
    let (normalized, offsets) = normalize_without_whitespace(source);
    for token in [
        format!("@group({group})@binding({binding})"),
        format!("@binding({binding})@group({group})"),
    ] {
        if let Some(start) = normalized.find(&token) {
            let end = start + token.len() - 1;
            return Some(Span::new(offsets[start] as u32, (offsets[end] + 1) as u32));
        }
    }
    None
}

fn normalize_without_whitespace(source: &str) -> (String, Vec<usize>) {
    let mut normalized = String::with_capacity(source.len());
    let mut offsets = Vec::with_capacity(source.len());
    let mut chars = source.char_indices().peekable();
    let mut line_comment = false;
    let mut block_comment_depth = 0usize;

    while let Some((offset, ch)) = chars.next() {
        let next = chars.peek().map(|(_, next)| *next);

        if line_comment {
            if ch == '\n' {
                line_comment = false;
            }
            continue;
        }

        if block_comment_depth > 0 {
            if ch == '/' && next == Some('*') {
                block_comment_depth += 1;
                chars.next();
            } else if ch == '*' && next == Some('/') {
                block_comment_depth -= 1;
                chars.next();
            }
            continue;
        }

        if ch == '/' && next == Some('/') {
            line_comment = true;
            chars.next();
            continue;
        }

        if ch == '/' && next == Some('*') {
            block_comment_depth = 1;
            chars.next();
            continue;
        }

        if !ch.is_whitespace() {
            let start = normalized.len();
            normalized.push(ch);
            let width = normalized.len() - start;
            for _ in 0..width {
                offsets.push(offset);
            }
        }
    }
    (normalized, offsets)
}

fn stage_name(stage: ShaderStage) -> &'static str {
    match stage {
        ShaderStage::Vertex => "vertex",
        ShaderStage::Fragment => "fragment",
        ShaderStage::Compute => "compute",
    }
}

fn stage_attribute(stage: ShaderStage) -> &'static str {
    match stage {
        ShaderStage::Vertex => "@vertex",
        ShaderStage::Fragment => "@fragment",
        ShaderStage::Compute => "@compute",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen2d_body(fragment_body: &str, extra_items: &str) -> String {
        format!(
            r#"
@vertex
fn vs_main(in: VzglydVertexInput) -> VzglydVertexOutput {{
    var out: VzglydVertexOutput;
    out.clip_pos = vec4<f32>(in.position, 1.0);
    out.tex_coords = in.tex_coords;
    out.color = in.color;
    out.mode = in.mode;
    return out;
}}

@fragment
fn fs_main(in: VzglydVertexOutput) -> @location(0) vec4<f32> {{
    {fragment_body}
}}

{extra_items}
"#
        )
    }

    fn screen2d_shader(bindings: &str, fragment_body: &str, extra_items: &str) -> String {
        format!(
            r#"
struct VSIn {{
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) mode: f32,
}};

struct VSOut {{
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) mode: f32,
}};

@vertex
fn vs_main(in: VSIn) -> VSOut {{
    var out: VSOut;
    out.pos = vec4<f32>(in.pos, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    out.mode = in.mode;
    return out;
}}

{bindings}

@fragment
fn fs_main(in: VSOut) -> @location(0) vec4<f32> {{
    {fragment_body}
}}

{extra_items}
"#
        )
    }

    fn valid_screen2d_bindings() -> &'static str {
        r#"
@group(0) @binding(0) var t_diffuse: texture_2d<f32>;
@group(0) @binding(1) var t_font: texture_2d<f32>;
@group(0) @binding(2) var t_detail: texture_2d<f32>;
@group(0) @binding(3) var t_lookup: texture_2d<f32>;
@group(0) @binding(4) var s_diffuse: sampler;
@group(0) @binding(5) var s_font: sampler;

struct TimeUniform {
    time: f32,
}

@group(0) @binding(6) var<uniform> u: TimeUniform;
"#
    }

    #[test]
    fn imported_scene_shader_body_matches_prelude_contract() {
        default_imported_scene_shader_source()
            .expect("default imported scene shader should validate");
    }

    #[test]
    fn slide_shader_body_is_prepended_with_prelude() {
        let body = screen2d_body("return vec4<f32>(u.time, in.color.yzw);", "");
        let shader = validate_slide_shader_body(
            "screen_body.wgsl",
            &body,
            ShaderContract::Screen2D,
            "vs_main",
            "fs_main",
        )
        .expect("body-only shader should validate");

        assert!(shader.contains("const VZGLYD_SHADER_CONTRACT_VERSION: u32 = 1u;"));
        assert!(shader.contains("@group(0) @binding(6) var<uniform> u: VzglydUniforms;"));
    }

    #[test]
    fn parse_errors_report_line_numbers() {
        let shader = screen2d_shader(
            valid_screen2d_bindings(),
            "let broken = ;\nreturn in.color;",
            "",
        );

        let error = validate_shader_source(
            "broken_parse.wgsl",
            &shader,
            ShaderContract::Screen2D,
            "vs_main",
            "fs_main",
        )
        .expect_err("broken shader should fail parsing");

        let location = error
            .location()
            .expect("parse error should report a location");
        assert!(location.line_number > 0);
        assert!(error.diagnostic().contains("broken_parse.wgsl"));
        assert!(error.diagnostic().contains("error"));
    }

    #[test]
    fn compute_entry_points_are_rejected() {
        let shader = screen2d_shader(
            valid_screen2d_bindings(),
            "return in.color;",
            "@compute @workgroup_size(1) fn cs_main() {}",
        );

        let error = validate_shader_source(
            "compute.wgsl",
            &shader,
            ShaderContract::Screen2D,
            "vs_main",
            "fs_main",
        )
        .expect_err("compute entry points should be rejected");

        assert!(error.summary().contains("compute entry point"));
        assert!(error.location().is_some());
    }

    #[test]
    fn storage_buffers_are_rejected() {
        let bindings = r#"
struct StorageData {
    value: f32,
}

@group(0) @binding(0) var<storage, read> data: StorageData;
"#;
        let shader = screen2d_shader(bindings, "return in.color;", "");

        let error = validate_shader_source(
            "storage.wgsl",
            &shader,
            ShaderContract::Screen2D,
            "vs_main",
            "fs_main",
        )
        .expect_err("storage buffers should be rejected");

        assert!(error.summary().contains("storage buffers"));
        assert!(error.location().is_some());
    }

    #[test]
    fn bind_group_mismatches_are_rejected() {
        let bindings = r#"
@group(0) @binding(0) var t_diffuse: sampler;
"#;
        let shader = screen2d_shader(bindings, "return in.color;", "");

        let error = validate_shader_source(
            "binding_mismatch.wgsl",
            &shader,
            ShaderContract::Screen2D,
            "vs_main",
            "fs_main",
        )
        .expect_err("binding mismatch should be rejected");

        assert!(error.summary().contains("@group(0) @binding(0)"));
        assert!(error.summary().contains("texture_2d"));
        assert!(error.location().is_some());
    }

    #[test]
    fn invalid_custom_shader_is_rejected() {
        let shader = screen2d_body(
            "return in.color;",
            "@group(1) @binding(0) var bad_tex: texture_2d<f32>;",
        );
        let error = validate_slide_shader_body(
            "invalid_custom_shader.wgsl",
            &shader,
            ShaderContract::Screen2D,
            "vs_main",
            "fs_main",
        )
        .expect_err("invalid custom shader should be rejected");

        assert!(error.summary().contains("may only use bind group 0"));
    }

    #[test]
    fn prelude_bindings_cannot_be_redeclared_in_shader_body() {
        let shader = screen2d_body(
            "return in.color;",
            "@group(0) @binding(0) var another_tex: texture_2d<f32>;",
        );
        let error = validate_slide_shader_body(
            "binding_conflict.wgsl",
            &shader,
            ShaderContract::Screen2D,
            "vs_main",
            "fs_main",
        )
        .expect_err("reserved prelude bindings should be rejected");

        assert!(
            error
                .summary()
                .contains("reserved by the VZGLYD shader prelude")
        );
        assert!(error.location().is_some());
    }
}
