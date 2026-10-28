// Fills only empty destination air, behind the block scene and inside its native mesh clip.
fn shade(input: MaterialInput) -> Surface {
    let v = clamp(input.uv.y, 0.0, 1.0);
    let day = param(input, 0u) > 0.5;
    let horizon = select(vec3<f32>(0.19,0.035,0.015), vec3<f32>(0.65,0.79,0.92), day);
    let top = select(vec3<f32>(0.07,0.012,0.009), vec3<f32>(0.22,0.49,0.88), day);
    let color = mix(horizon, top, smoothstep(0.3, 0.85, v));
    return Surface(color, 1.0, vec3<f32>(0.0));
}
