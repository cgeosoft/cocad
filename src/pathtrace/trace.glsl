// Fragment shader of the GPU path tracer. It renders one sample of each
// pixel in the tile. It follows `Scene::trace` and `Scene::hit_mesh` in
// `pathtrace.rs`: keep the two in step. `gpu.rs` puts `#version` and
// `MAX_PLANES` in front of this text.

uniform usampler2D u_tris;
uniform usampler2D u_nodes;
uniform usampler2D u_paint;
uniform usampler2D u_clip;

uniform ivec2 u_size;
uniform uint u_pass;

uniform vec3 u_eye;
uniform vec3 u_fwd;
uniform vec3 u_right;
uniform vec3 u_up;
uniform float u_th;
uniform float u_aspect;

uniform vec3 u_color;
uniform float u_rough;
uniform float u_layer;

uniform vec3 u_horizon;
uniform vec3 u_zenith;
uniform float u_top;
uniform vec3 u_floor_albedo;
uniform float u_floor;
uniform vec3 u_center;
uniform float u_radius;
uniform vec3 u_key_dir;
uniform float u_key_cos;
uniform float u_key_rad;

uniform int u_npaint;
uniform int u_nclip;
uniform int u_nplanes;
uniform vec4 u_planes[MAX_PLANES];

out vec4 o_color;

const float EPS = 1e-4;
const float FMAX = 3.4028235e38;
const float PI = 3.14159265;
const float TAU = 6.28318531;
// Texels in a row of the triangle and node textures (`TEX_W` in gpu.rs).
const int TEX_W = 4096;

// ------------------------------------------------------------ random ----

uint g_rng;

uint pcg(uint v) {
    uint s = v * 747796405u + 2891336453u;
    uint w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

float rnd() {
    g_rng = pcg(g_rng);
    return float(g_rng >> 8u) / 16777216.0;
}

float sgn(float v) {
    return v >= 0.0 ? 1.0 : -1.0;
}

// --------------------------------------------------------------- scene ----

struct Tri {
    vec3 v0;
    vec3 e1;
    vec3 e2;
    vec3 g;
    uint id;
};

uvec4 tri_texel(uint i, int k) {
    int t = int(i) * 4 + k;
    return texelFetch(u_tris, ivec2(t % TEX_W, t / TEX_W), 0);
}

Tri load_tri(uint i) {
    uvec4 a = tri_texel(i, 0);
    uvec4 b = tri_texel(i, 1);
    uvec4 c = tri_texel(i, 2);
    uvec4 d = tri_texel(i, 3);
    Tri t;
    t.v0 = uintBitsToFloat(a.xyz);
    t.id = a.w;
    t.e1 = uintBitsToFloat(b.xyz);
    t.e2 = uintBitsToFloat(c.xyz);
    t.g = uintBitsToFloat(uvec3(b.w, c.w, d.w));
    return t;
}

// The decode of `oct` in gpu.rs.
vec3 unoct(uint p) {
    vec2 e = vec2(float(int(p << 16u) >> 16), float(int(p) >> 16)) / 32767.0;
    vec3 n = vec3(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        n.xy = (1.0 - abs(n.yx)) * vec2(sgn(n.x), sgn(n.y));
    }
    return normalize(n);
}

// The smooth shading normal at (u, v).
vec3 tri_normal(uint i, float u, float v) {
    uvec4 d = tri_texel(i, 3);
    return normalize(unoct(d.x) * (1.0 - u - v) + unoct(d.y) * u + unoct(d.z) * v);
}

void load_node(uint i, out vec3 mn, out vec3 mx, out uint start, out uint count) {
    int t = int(i) * 2;
    uvec4 a = texelFetch(u_nodes, ivec2(t % TEX_W, t / TEX_W), 0);
    uvec4 b = texelFetch(u_nodes, ivec2((t + 1) % TEX_W, (t + 1) / TEX_W), 0);
    mn = uintBitsToFloat(a.xyz);
    start = a.w;
    mx = uintBitsToFloat(b.xyz);
    count = b.w;
}

// The range of triangle `tri` in a range texture: the first entry with an
// end above `tri` (`partition_point`), or `n` after the last range.
int range_of(usampler2D t, int n, uint tri) {
    int lo = 0;
    int hi = n;
    while (lo < hi) {
        int mid = (lo + hi) / 2;
        if (texelFetch(t, ivec2(mid, 0), 0).x <= tri) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    return lo;
}

// `Paint::color`, or the color of the look.
vec3 albedo_of(uint id) {
    int k = range_of(u_paint, u_npaint, id);
    if (k < u_npaint) {
        return uintBitsToFloat(texelFetch(u_paint, ivec2(k, 0), 0).yzw);
    }
    return u_color;
}

// `Clip::offset`.
vec3 offset_of(uint id) {
    int k = range_of(u_clip, u_nclip, id);
    if (k < u_nclip) {
        return uintBitsToFloat(texelFetch(u_clip, ivec2(k, 0), 0).yzw);
    }
    return vec3(0.0);
}

// `Clip::keeps`.
bool keeps(vec3 p, uint id) {
    vec3 q = p - offset_of(id);
    for (int k = 0; k < u_nplanes; k++) {
        if (dot(u_planes[k].xyz, q) + u_planes[k].w < 0.0) {
            return false;
        }
    }
    return true;
}

// `Clip::entry`: the t where the ray goes into the kept space, or -1.
float entry(vec3 o, vec3 d, uint id, out vec3 normal) {
    vec3 q = o - offset_of(id);
    float best = 0.0;
    bool found = false;
    normal = vec3(0.0);
    for (int k = 0; k < u_nplanes; k++) {
        vec3 n = u_planes[k].xyz;
        float nd = dot(n, d);
        if (nd > 0.0) {
            float t = -(dot(n, q) + u_planes[k].w) / nd;
            if (!found || t > best) {
                best = t;
                found = true;
                normal = length(n) > 0.0 ? -normalize(n) : vec3(0.0);
            }
        }
    }
    return (found && best > 0.0) ? best : -1.0;
}

bool slab(vec3 mn, vec3 mx, vec3 o, vec3 inv, float t_max) {
    vec3 t0 = (mn - o) * inv;
    vec3 t1 = (mx - o) * inv;
    vec3 lo = min(t0, t1);
    vec3 hi = max(t0, t1);
    float tmin = max(max(max(lo.x, lo.y), lo.z), 0.0);
    float tmax = min(min(min(hi.x, hi.y), hi.z), t_max);
    return tmin <= tmax;
}

bool intersect(Tri t, vec3 o, vec3 d, out float tt, out float u, out float v) {
    tt = 0.0;
    u = 0.0;
    v = 0.0;
    vec3 p = cross(d, t.e2);
    float det = dot(t.e1, p);
    if (abs(det) < 1e-12) {
        return false;
    }
    float inv = 1.0 / det;
    vec3 s = o - t.v0;
    // A small tolerance closes the hairline gaps between long thin
    // neighbour triangles.
    const float TOL = 1e-5;
    u = dot(s, p) * inv;
    if (!(u >= -TOL && u <= 1.0 + TOL)) {
        return false;
    }
    vec3 q = cross(s, t.e1);
    v = dot(d, q) * inv;
    if (v < -TOL || u + v > 1.0 + TOL) {
        return false;
    }
    // Distance from the face plane, as on the CPU.
    float gd = dot(t.g, d);
    tt = abs(gd) > 1e-4 ? dot(t.g, t.v0 - o) / gd : dot(t.e2, q) * inv;
    return tt > EPS;
}

struct Hit {
    bool ok;
    float t;
    uint i;
    float u;
    float v;
};

// `Scene::hit_mesh`: the nearest model hit before `t_max`.
Hit hit_mesh(vec3 o, vec3 d, float t_max) {
    vec3 inv = 1.0 / d;
    Hit best = Hit(false, t_max, 0u, 0.0, 0.0);
    float t_best = t_max;
    bool cut = u_nplanes > 0;
    bool best_back = false;
    uint stack[64];
    stack[0] = 0u;
    int sp = 1;
    while (sp > 0) {
        sp--;
        vec3 mn;
        vec3 mx;
        uint start;
        uint count;
        load_node(stack[sp], mn, mx, start, count);
        float tie = cut ? EPS * (1.0 + abs(t_best)) : 0.0;
        if (!slab(mn, mx, o, inv, t_best + tie)) {
            continue;
        }
        if (count > 0u) {
            for (uint i = start; i < start + count; i++) {
                Tri tri = load_tri(i);
                float t;
                float u;
                float v;
                if (!intersect(tri, o, d, t, u, v)) {
                    continue;
                }
                bool take;
                bool back = dot(tri.g, d) > 0.0;
                if (!cut) {
                    take = t < t_best;
                } else {
                    // A back face wins a near tie with a front face.
                    bool near = abs(t - t_best) <= EPS * (1.0 + abs(t_best));
                    take = (t < t_best || (near && back && !best_back))
                        && !(near && best_back && !back)
                        && keeps(o + d * t, tri.id);
                }
                if (take) {
                    t_best = t;
                    best_back = back;
                    best = Hit(true, t, i, u, v);
                }
            }
        } else if (sp + 2 <= 64) {
            stack[sp] = start;
            stack[sp + 1] = start + 1u;
            sp += 2;
        }
    }
    return best;
}

bool occluded(vec3 o, vec3 d) {
    return hit_mesh(o, d, FMAX).ok;
}

// ----------------------------------------------------------- sampling ----

// glam `any_orthonormal_vector` and its cross product.
void basis(vec3 n, out vec3 t, out vec3 b) {
    float s = n.z >= 0.0 ? 1.0 : -1.0;
    float a = -1.0 / (s + n.z);
    float bb = n.x * n.y * a;
    t = vec3(bb, s + n.y * n.y * a, -n.y);
    b = cross(n, t);
}

vec3 sample_cosine(vec3 n) {
    float u1 = rnd();
    float u2 = rnd();
    float r = sqrt(u1);
    float phi = TAU * u2;
    vec3 t;
    vec3 b;
    basis(n, t, b);
    return normalize(t * (r * cos(phi)) + b * (r * sin(phi)) + n * sqrt(max(1.0 - u1, 0.0)));
}

vec3 sample_lobe(vec3 dir, float ex) {
    float u1 = rnd();
    float u2 = rnd();
    float cos_t = pow(u1, 1.0 / (ex + 1.0));
    float sin_t = sqrt(max(1.0 - cos_t * cos_t, 0.0));
    float phi = TAU * u2;
    vec3 t;
    vec3 b;
    basis(dir, t, b);
    return normalize(t * (sin_t * cos(phi)) + b * (sin_t * sin(phi)) + dir * cos_t);
}

vec3 sample_cone(vec3 dir, float cos_max) {
    float u1 = rnd();
    float u2 = rnd();
    float cos_t = 1.0 - u1 * (1.0 - cos_max);
    float sin_t = sqrt(max(1.0 - cos_t * cos_t, 0.0));
    float phi = TAU * u2;
    vec3 t;
    vec3 b;
    basis(dir, t, b);
    return t * (sin_t * cos(phi)) + b * (sin_t * sin(phi)) + dir * cos_t;
}

// ---------------------------------------------------------------- trace ----

// `Scene::env`.
vec3 env(vec3 d, bool key) {
    float up = max(d.z, 0.0);
    float up3 = up * up * up;
    vec3 c = mix(u_horizon, u_zenith, sqrt(up)) + vec3(u_top * up3 * up3);
    if (key && dot(d, u_key_dir) > u_key_cos) {
        c += vec3(u_key_rad);
    }
    return c;
}

float max3(vec3 v) {
    return max(v.x, max(v.y, v.z));
}

// `Scene::trace`.
vec3 trace(vec3 o, vec3 d) {
    vec3 l = vec3(0.0);
    vec3 thr = vec3(1.0);
    bool specular = true;
    for (int bounce = 0; bounce < 6; bounce++) {
        float floor_t = (d.z < 0.0 && o.z > u_floor) ? (u_floor - o.z) / d.z : FMAX;
        Hit h = hit_mesh(o, d, floor_t);
        vec3 p;
        vec3 n;
        vec3 gn;
        vec3 albedo;
        float rough;
        float spec;
        if (h.ok) {
            Tri tri = load_tri(h.i);
            // The cut face in front of the hit (`Scene::cap`).
            float tc = -1.0;
            vec3 cap = vec3(0.0);
            if (u_nplanes > 0 && dot(tri.g, d) > 0.0) {
                tc = entry(o, d, tri.id, cap);
                if (tc >= h.t) {
                    tc = -1.0;
                }
            }
            albedo = albedo_of(tri.id);
            rough = u_rough;
            spec = 0.05;
            if (tc > 0.0) {
                // The ray goes into the solid through the cut face.
                p = o + d * tc;
                n = cap;
                gn = cap;
            } else {
                gn = tri.g;
                n = tri_normal(h.i, h.u, h.v);
                p = o + d * h.t;
                if (u_layer > 0.0 && abs(n.z) < 0.97) {
                    // Ripple the normal up and down along the side: printed layers.
                    vec3 up = normalize(vec3(0.0, 0.0, 1.0) - n * n.z);
                    float phase = (p.z - u_floor) / u_layer * TAU;
                    n = normalize(n + up * (0.35 * sin(phase)));
                }
            }
        } else if (floor_t < FMAX) {
            p = o + d * floor_t;
            // Far floor fades into the backdrop.
            float dist = length((p - u_center).xy) / u_radius;
            float fade = clamp((dist - 4.0) / 6.0, 0.0, 1.0);
            if (fade > 0.0) {
                l += thr * fade * env(normalize(vec3(d.x, d.y, 0.02)), false);
                thr *= 1.0 - fade;
                if (max3(thr) < 1e-3) {
                    break;
                }
            }
            n = vec3(0.0, 0.0, 1.0);
            gn = n;
            albedo = u_floor_albedo;
            rough = 0.6;
            spec = 0.02;
        } else {
            l += thr * env(d, specular);
            break;
        }
        // Face the ray.
        if (dot(gn, d) > 0.0) {
            gn = -gn;
            n = -n;
        }
        p += gn * EPS * (1.0 + max3(abs(p)) * 1e-4);

        // Direct key light.
        vec3 ld = sample_cone(u_key_dir, u_key_cos);
        float cos_l = dot(n, ld);
        if (cos_l > 0.0 && dot(gn, ld) > 0.0 && !occluded(p, ld)) {
            float omega = TAU * (1.0 - u_key_cos);
            l += thr * albedo * (1.0 / PI) * u_key_rad * cos_l * omega * (1.0 - spec);
        }

        // Next direction: glossy coat or diffuse base.
        float cos_v = clamp(dot(-d, n), 0.0, 1.0);
        float m = 1.0 - cos_v;
        float fresnel = spec + (1.0 - spec) * (m * m * m * m * m);
        if (rnd() < fresnel) {
            vec3 r = reflect(d, n);
            float ex = 2.0 / (rough * rough * rough + 1e-3);
            d = sample_lobe(r, ex);
            if (dot(d, gn) <= 0.0) {
                break;
            }
            specular = rough < 0.4;
        } else {
            d = sample_cosine(n);
            if (dot(d, gn) <= 0.0) {
                d = sample_cosine(gn);
            }
            thr *= albedo;
            specular = false;
        }
        o = p;

        if (bounce >= 2) {
            float q = min(max3(thr), 0.95);
            if (rnd() > q) {
                break;
            }
            thr /= q;
        }
    }
    return l;
}

void main() {
    ivec2 px = ivec2(gl_FragCoord.xy);
    // Image row 0 is the top row; GL row 0 is the bottom row.
    int x = px.x;
    int y = u_size.y - 1 - px.y;
    g_rng = pcg(uint(y * u_size.x + x) + pcg(u_pass + 0x2545F491u));
    float sx = ((float(x) + rnd()) / float(u_size.x) * 2.0 - 1.0) * u_th * u_aspect;
    float sy = (1.0 - (float(y) + rnd()) / float(u_size.y) * 2.0) * u_th;
    vec3 d = normalize(u_fwd + u_right * sx + u_up * sy);
    vec3 c = trace(u_eye, d);
    if (any(isnan(c)) || any(isinf(c))) {
        c = vec3(0.0);
    }
    // Clamp fireflies.
    o_color = vec4(min(c, vec3(20.0)), 1.0);
}
