@vertex
fn vs_main(in: VzglydVertexInput) -> VzglydVertexOutput {
    var out: VzglydVertexOutput;
    out.clip_pos = u.view_proj * vec4<f32>(in.position, 1.0);
    out.world_pos = in.position;
    out.normal = normalize(in.normal);
    out.color = in.color;
    out.mode = in.mode;
    return out;
}

fn apply_fog(rgb: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let dist = length(world_pos - u.cam_pos);
    let t = clamp((dist - u.fog_start) / (u.fog_end - u.fog_start), 0.0, 1.0);
    let fog_f = t * t * (3.0 - 2.0 * t);
    return mix(rgb, u.fog_color.rgb, fog_f);
}

fn sky_at(_dir: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(0.0, 0.0, 0.0);
}

fn surface_pattern(world_pos: vec3<f32>) -> vec3<f32> {
    let uv_a = world_pos.xz * 0.065;
    let uv_b = world_pos.xz * 0.025 + vec2<f32>(11.3, 7.1);
    let a = textureSample(t_material_a, s_repeat, uv_a).rgb;
    let b = textureSample(t_material_b, s_repeat, uv_b).rgb;
    return mix(a, b, 0.45);
}

fn loading_light_dir() -> vec3<f32> {
    let base = vzglyd_main_light_dir();
    let orbit = normalize(vec3<f32>(
        cos(u.time * 0.33) * 0.78,
        0.35 + 0.25 * sin(u.time * 0.21),
        sin(u.time * 0.33) * 0.78,
    ));
    return normalize(base * 0.20 + orbit * 0.80);
}

fn loading_light_rgb() -> vec3<f32> {
    let tint = 0.5 + 0.5 * sin(u.time * 0.17 + 1.2);
    let cool = vec3<f32>(0.52, 0.72, 1.0);
    let warm = vec3<f32>(1.0, 0.90, 0.74);
    return mix(cool, warm, tint) * max(vzglyd_main_light_strength(), 0.85);
}

fn loading_ambient_rgb() -> vec3<f32> {
    return max(vzglyd_ambient_light(), vec3<f32>(0.05, 0.05, 0.06));
}

fn lit_surface(base_color: vec4<f32>, normal: vec3<f32>, world_pos: vec3<f32>) -> vec4<f32> {
    let detail = textureSample(t_noise, s_repeat, world_pos.xz * 0.08).rg * 2.0 - 1.0;
    let perturbed = normalize(normal + vec3<f32>(detail.x, 0.0, detail.y) * 0.30);
    let view_dir = normalize(u.cam_pos - world_pos);
    let light_dir = loading_light_dir();
    let diff = max(dot(perturbed, light_dir), 0.0);
    let band = floor(diff * 3.5 + 0.5) / 3.5;
    let rim = pow(1.0 - max(dot(perturbed, view_dir), 0.0), 2.0) * 0.16;
    let light = loading_ambient_rgb() + loading_light_rgb() * band * 0.92;
    let albedo = base_color.rgb * mix(vec3<f32>(0.88, 0.90, 0.94), surface_pattern(world_pos), 0.20);
    return vec4<f32>(
        apply_fog(albedo * light + rim * (0.08 + loading_light_rgb() * 0.12), world_pos),
        base_color.a,
    );
}

@fragment
fn fs_main(in: VzglydVertexOutput) -> @location(0) vec4<f32> {
    let material_mode = in.mode;
    let base = in.color;

    if material_mode >= 3.5 {
        let uv0 = in.world_pos.xz * 0.06 + vec2<f32>(u.time * 0.030, -u.time * 0.021);
        let uv1 = in.world_pos.xz * 0.11 + vec2<f32>(-u.time * 0.014, u.time * 0.018);
        let n0 = textureSample(t_noise, s_repeat, uv0).rg * 2.0 - 1.0;
        let n1 = textureSample(t_material_b, s_repeat, uv1).rg * 2.0 - 1.0;
        let water_n = normalize(vec3<f32>((n0.x + n1.x) * 0.45, 1.0, (n0.y + n1.y) * 0.45));
        let view_dir = normalize(u.cam_pos - in.world_pos);
        let fresnel = pow(1.0 - max(dot(water_n, view_dir), 0.0), 3.0);
        let sparkle = pow(max(dot(reflect(-loading_light_dir(), water_n), view_dir), 0.0), 72.0);
        let water_base = mix(base.rgb, surface_pattern(in.world_pos), 0.35);
        let water_col = water_base * (loading_ambient_rgb() + loading_light_rgb() * 0.28)
            + loading_light_rgb() * sparkle * 0.60
            + fresnel * 0.04;
        return vec4<f32>(apply_fog(water_col, in.world_pos), max(base.a, 0.45));
    }

    var shaded = lit_surface(base, in.normal, in.world_pos);
    if material_mode >= 2.5 {
        let pulse = 0.70 + 0.30 * sin(u.time * 1.6);
        let emissive = base.rgb * (1.05 + pulse);
        return vec4<f32>(apply_fog(emissive, in.world_pos), base.a);
    }
    if material_mode >= 1.5 {
        shaded.a = base.a * 0.55;
        return shaded;
    }
    if material_mode >= 0.5 {
        if base.a < 0.5 {
            discard;
        }
        shaded.a = 1.0;
        return shaded;
    }
    return shaded;
}
