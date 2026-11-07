use std::f32::consts::TAU;

/// Renders the custom "Anti-Burn OLED Guard" icon into a top-down 32-bit BGRA (`0xAARRGGBB`) pixel buffer of `size x size`.
/// Design:
/// - Sleek Windows 11 dark-obsidian squircle shield (`#07090E`) with a luminous cyan-indigo-magenta metallic rim
/// - Central pure-black OLED Eclipse core (`#000000`) symbolizing OLED pixel protection
/// - Iridescent plasma metaball corona & orbital flow arcs around the eclipse (matching the screensaver)
pub fn render_icon_bgra(size: usize) -> Vec<u32> {
    let mut out = vec![0u32; size * size];
    let s_f = size as f32;
    // Use 3x3 supersampling on <=64px icons and 2x2 on 256px for razor-sharp anti-aliased edges
    let ss = if size <= 64 { 3 } else { 2 };
    let inv_ss = 1.0 / (ss as f32);
    let sample_weight = 1.0 / ((ss * ss) as f32);

    // Boost stroke thickness slightly at small sizes (16x16, 24x24, 32x32) so tray & titlebar icons pop clearly
    let small_boost = if size <= 20 {
        1.45
    } else if size <= 32 {
        1.22
    } else {
        1.0
    };

    for py in 0..size {
        for px in 0..size {
            let mut acc_r = 0.0f32;
            let mut acc_g = 0.0f32;
            let mut acc_b = 0.0f32;
            let mut acc_a = 0.0f32;

            for sy in 0..ss {
                for sx in 0..ss {
                    let fx = (px as f32 + (sx as f32 + 0.5) * inv_ss) / s_f;
                    let fy = (py as f32 + (sy as f32 + 0.5) * inv_ss) / s_f;

                    // Normalized coordinates in [-1.0, 1.0]
                    let nx = (fx - 0.5) * 2.0;
                    let ny = (fy - 0.5) * 2.0;

                    // 1. Squircle outer boundary (L^4.2 superellipse, radius 0.90)
                    let ax = (nx / 0.91).abs();
                    let ay = (ny / 0.91).abs();
                    let squircle_d = (ax.powf(4.2) + ay.powf(4.2)).powf(1.0 / 4.2) - 1.0;
                    // Convert normalized distance to approximate pixel distance for crisp AA
                    let edge_aa = (0.5 - squircle_d * (s_f * 0.45)).clamp(0.0, 1.0);
                    if edge_aa <= 0.001 {
                        continue;
                    }

                    // Base obsidian background with subtle diagonal deep-indigo ambient gradient
                    let bg_grad = ((nx * -0.4 + ny * -0.6) * 0.5 + 0.5).clamp(0.0, 1.0);
                    let mut r = 8.0 + bg_grad * 12.0;
                    let mut g = 10.0 + bg_grad * 16.0;
                    let mut b = 16.0 + bg_grad * 28.0;

                    // 2. Subtle inner shield contour glow
                    let sh_x = nx.abs();
                    let sh_y = ny + 0.04;
                    let shield_w = (0.72 - (sh_y + 0.55).max(0.0) * 0.36).max(0.08);
                    let shield_dist = ((sh_x - shield_w).max(0.0).powi(2)
                        + (sh_y.abs() - 0.64).max(0.0).powi(2))
                    .sqrt()
                        + (sh_x - shield_w).max(sh_y.abs() - 0.64).min(0.0);
                    let shield_rim =
                        (-((shield_dist.abs() / (0.045 * small_boost)).powi(2))).exp() * 0.38;
                    r += shield_rim * 56.0;
                    g += shield_rim * 189.0;
                    b += shield_rim * 248.0;

                    // 3. Central OLED Eclipse + Coupled Metaball Corona Ring
                    let rad = (nx * nx + ny * ny).sqrt();
                    let angle = ny.atan2(nx);

                    // Three orbiting metaball lobes around the eclipse ring (radius ~0.46)
                    let ring_r = 0.44;
                    let mut meta_field = 0.0f32;
                    let lobes: [(f32, f32, [f32; 3]); 3] = [
                        (-0.65, 0.17, [56.0, 215.0, 255.0]),  // Electric Cyan
                        (1.45, 0.16, [168.0, 85.0, 247.0]),   // Neon Violet
                        (3.65, 0.15, [244.0, 63.0, 140.0]),   // Plasma Rose-Magenta
                    ];

                    let mut lobe_r = 0.0f32;
                    let mut lobe_g = 0.0f32;
                    let mut lobe_b = 0.0f32;

                    for &(ang, l_rad, col) in &lobes {
                        let lx = ang.cos() * ring_r;
                        let ly = ang.sin() * ring_r;
                        let d2 = (nx - lx).powi(2) + (ny - ly).powi(2) + 0.012;
                        let w = (l_rad * l_rad * small_boost) / d2;
                        meta_field += w;
                        lobe_r += col[0] * w;
                        lobe_g += col[1] * w;
                        lobe_b += col[2] * w;
                    }

                    // Continuous plasma corona ring around the eclipse core
                    let ring_delta = (rad - ring_r).abs();
                    let ring_sigma = 0.075 * small_boost;
                    let corona = (-(ring_delta * ring_delta) / (ring_sigma * ring_sigma)).exp();

                    // Angular color sweep around the corona (Cyan -> Violet -> Magenta -> Cyan)
                    let t_ang = (angle / TAU + 0.5).fract();
                    let (cr, cg, cb) = corona_palette(t_ang);

                    let inv_mf = 1.0 / (meta_field + 0.001);
                    let mr = lobe_r * inv_mf;
                    let mg = lobe_g * inv_mf;
                    let mb = lobe_b * inv_mf;

                    let ring_intensity = (corona * 0.95 + (meta_field * 0.28).min(0.85)).min(1.35);
                    r += (cr * 0.65 + mr * 0.35) * ring_intensity;
                    g += (cg * 0.65 + mg * 0.35) * ring_intensity;
                    b += (cb * 0.65 + mb * 0.35) * ring_intensity;

                    // Bright white-cyan hot core along the inner eclipse rim
                    let inner_rim_delta = (rad - 0.36).abs();
                    let inner_rim = (-(inner_rim_delta / (0.028 * small_boost)).powi(2)).exp();
                    let crescent_bias = ((nx * -0.7 + ny * -0.7) * 0.5 + 0.65).clamp(0.25, 1.15);
                    r += 210.0 * inner_rim * crescent_bias;
                    g += 245.0 * inner_rim * crescent_bias;
                    b += 255.0 * inner_rim * crescent_bias;

                    // 4. Pure-Black OLED Total Eclipse Core (`#000000` inside rad < 0.34)
                    // Symbolizes 100% turned-off OLED subpixels protected against burn-in!
                    let core_edge = ((rad - 0.345) / (0.025 * small_boost)).clamp(0.0, 1.0);
                    let core_mask = core_edge * core_edge * (3.0 - 2.0 * core_edge);
                    r *= core_mask;
                    g *= core_mask;
                    b *= core_mask;

                    // Subtle tiny specular reflection dot inside the top-left of the dark eclipse lens
                    let lens_dx = nx + 0.13;
                    let lens_dy = ny + 0.13;
                    let lens_spec = (-((lens_dx * lens_dx + lens_dy * lens_dy) / 0.006)).exp()
                        * (1.0 - core_mask)
                        * 0.55;
                    r += 110.0 * lens_spec;
                    g += 210.0 * lens_spec;
                    b += 255.0 * lens_spec;

                    // 5. Outer Metallic Squircle Bezel Rim (Cyan top-left highlight -> Violet bottom-right)
                    let rim_dist = squircle_d.abs();
                    let rim_width = 0.055 * small_boost;
                    let rim_glow = (-(rim_dist / rim_width).powi(2)).exp();
                    let rim_t = ((nx + ny) * 0.35 + 0.5).clamp(0.0, 1.0);
                    let rim_r = 56.0 * (1.0 - rim_t) + 168.0 * rim_t;
                    let rim_g = 210.0 * (1.0 - rim_t) + 85.0 * rim_t;
                    let rim_b = 255.0 * (1.0 - rim_t) + 247.0 * rim_t;

                    r = r * (1.0 - rim_glow * 0.75) + rim_r * rim_glow * 0.95;
                    g = g * (1.0 - rim_glow * 0.75) + rim_g * rim_glow * 0.95;
                    b = b * (1.0 - rim_glow * 0.75) + rim_b * rim_glow * 0.95;

                    acc_r += r.clamp(0.0, 255.0) * edge_aa * sample_weight;
                    acc_g += g.clamp(0.0, 255.0) * edge_aa * sample_weight;
                    acc_b += b.clamp(0.0, 255.0) * edge_aa * sample_weight;
                    acc_a += 255.0 * edge_aa * sample_weight;
                }
            }

            let a_u8 = acc_a.round().clamp(0.0, 255.0) as u32;
            if a_u8 > 0 {
                // Convert straight alpha to un-premultiplied RGB for ICO / DIB, or keep straight RGB
                let inv_a = 255.0 / acc_a.max(1.0);
                let r_u8 = (acc_r * inv_a).round().clamp(0.0, 255.0) as u32;
                let g_u8 = (acc_g * inv_a).round().clamp(0.0, 255.0) as u32;
                let b_u8 = (acc_b * inv_a).round().clamp(0.0, 255.0) as u32;
                out[py * size + px] = (a_u8 << 24) | (r_u8 << 16) | (g_u8 << 8) | b_u8;
            }
        }
    }

    out
}

#[inline(always)]
fn corona_palette(t: f32) -> (f32, f32, f32) {
    // Smooth 3-stop cyclic gradient: Electric Cyan -> Deep Neon Indigo/Violet -> Vibrant Rose-Magenta -> Electric Cyan
    let x = (t.fract() + 1.0).fract() * 3.0;
    if x < 1.0 {
        let f = x * x * (3.0 - 2.0 * x);
        (
            56.0 + (139.0 - 56.0) * f,
            215.0 + (92.0 - 215.0) * f,
            255.0 + (246.0 - 255.0) * f,
        )
    } else if x < 2.0 {
        let u = x - 1.0;
        let f = u * u * (3.0 - 2.0 * u);
        (
            139.0 + (244.0 - 139.0) * f,
            92.0 + (63.0 - 92.0) * f,
            246.0 + (145.0 - 246.0) * f,
        )
    } else {
        let u = x - 2.0;
        let f = u * u * (3.0 - 2.0 * u);
        (
            244.0 + (56.0 - 244.0) * f,
            63.0 + (215.0 - 63.0) * f,
            145.0 + (255.0 - 145.0) * f,
        )
    }
}
