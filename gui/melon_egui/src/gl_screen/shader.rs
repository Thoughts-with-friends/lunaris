//! The blit shader, and what it chooses between.
//!
//! # Two pictures, one fragment at a time
//!
//! Under an OpenGL renderer a frame holds two very different things:
//!
//! * **3D**, rasterised at `256n x 192n` — real detail, already at the
//!   resolution the internal-resolution setting bought.
//! * **2D**, drawn by the DS's own engine at 256x192 and composited by
//!   repeating each of its pixels across an `n x n` block — no more detail than
//!   the DS had, just bigger squares.
//!
//! Raising the internal resolution cannot help the second, and xBRZ is what
//! does — but only if xBRZ actually gets to run on it.
//!
//! So it does, on the CPU, at 256x192, exactly as it does under the software
//! renderer: [`super::capture`] takes one texel per block back off the GPU,
//! `crate::upscale` filters those 49 152 pixels, and the result comes back as a
//! second texture. This shader then picks, per fragment, between the two:
//!
//! * a DS pixel whose block is **uniform** is 2D, so the filtered texture wins;
//! * a block with detail inside it is 3D, so the renderer's own output wins.
//!
//! Which is both halves of what was asked for, and neither is an approximation:
//! the 2D is put through the same xBRZ the software renderer uses, and the 3D
//! keeps every pixel the GPU drew.

/// The quad: one triangle strip covering the destination rectangle, sampling
/// one layer of the core's array texture.
pub const VERTEX: &str = r#"#version 330 core
out vec2 uv;
uniform vec4 rect;      // x, y, w, h in clip space
uniform vec4 uv_rect;   // u0, v0, du, dv
void main() {
    // Corners of a triangle strip, from gl_VertexID alone.
    vec2 corner = vec2(float(gl_VertexID & 1), float((gl_VertexID >> 1) & 1));
    uv = uv_rect.xy + corner * uv_rect.zw;
    gl_Position = vec4(rect.xy + corner * rect.zw, 0.0, 1.0);
}"#;

/// The fragment shader described at the top of this module.
///
/// # Why the sampling is not inside a branch
///
/// Every texel is fetched before anything is decided, and the decision is a
/// `mix` at the end. Texture sampling with an implicit level of detail is only
/// defined under control flow that every fragment in a quad agrees on, and
/// "is this pixel 2D" is decided per fragment — so returning early would be
/// undefined, with drivers free to differ on the result.
pub const FRAGMENT: &str = r##"#version 330 core
in vec2 uv;
out vec4 colour;

// The renderer's own output: layer 0 the top screen, layer 1 the bottom.
uniform sampler2DArray screen;
uniform float layer;
// The CPU xBRZ of this screen's 2D content, or a 1x1 placeholder.
uniform sampler2D filtered;
uniform bool use_filtered;

// The DS's own screen, which is the grid its 2D layers live on however large
// the texture is.
const vec2 DS = vec2(256.0, 192.0);

// Rec. 2020 luma. What "these two texels are the same colour" is measured in,
// because brightness is what the eye reads as an edge.
const vec3 LUMA = vec3(0.2627, 0.6780, 0.0593);

// How much a DS pixel's block may vary inside itself and still count as 2D.
// Not zero: the compositor's blend of a 2D layer over a 3D one can leave a
// texel or two of rounding across a block, and calling that 3D would leave the
// sprite in front of it unfiltered.
const float BLOCK_TOLERANCE = 0.02;

float luma_distance(vec3 a, vec3 b) {
    return dot(abs(a - b), LUMA);
}

void main() {
    vec2 size = vec2(textureSize(screen, 0).xy);
    // How many texels one DS pixel occupies: the internal resolution.
    vec2 scale = size / DS;
    vec2 base = floor(uv * DS);

// A texel from inside this DS pixel's block, at `u` in 0..1 across it.
// `texelFetch` rather than `texture` so this sees exact texels whatever the
// user set `Screen filtering` to -- that setting belongs to the two samples
// below, which are the ones actually shown.
#define BLOCK_AT(ux, uy) texelFetch(screen, ivec3(ivec2(clamp( \
    (base + vec2(ux, uy)) * scale, vec2(0.0), size - 1.0)), int(layer)), 0).rgb

    // Is this DS pixel a solid block? 3D detail varies inside one; a composited
    // 2D pixel cannot. At 1x every probe lands on the same texel, so everything
    // is 2D -- which is right, because at 1x everything is at the DS's own
    // resolution.
    vec3 q0 = BLOCK_AT(0.15, 0.15);
    vec3 q1 = BLOCK_AT(0.85, 0.15);
    vec3 q2 = BLOCK_AT(0.15, 0.85);
    vec3 q3 = BLOCK_AT(0.85, 0.85);
    float inside = max(max(luma_distance(q0, q1), luma_distance(q0, q2)),
                       max(luma_distance(q0, q3), luma_distance(q1, q2)));

    // The renderer's own picture. Its final pass writes RGBA -- unlike the
    // software rasteriser, whose host framebuffers are BGRA and are swizzled on
    // the CPU (see `app::to_image`). Sampling it in any other order turns the
    // DS's blue sky orange, which is what this used to do.
    vec3 plain = texture(screen, vec3(uv, layer)).rgb;
    // The same screen with its 2D put through xBRZ. Sampled with the same `uv`,
    // because `capture` reads it back in the texture's own orientation and it
    // is uploaded that way -- so the two agree and nothing has to be flipped.
    vec3 sharp = texture(filtered, uv).rgb;

    colour = vec4(
        mix(plain, sharp, float(use_filtered) * step(inside, BLOCK_TOLERANCE)),
        1.0
    );
}"##;
