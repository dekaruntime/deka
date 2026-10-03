//! macOS interop: one IOSurface, three views of it.
//!
//! * a `CVPixelBuffer` (what GPUI's `surface()` element takes),
//! * two `MTLTexture`s over its planes, created on *wgpu's* `MTLDevice`,
//! * wrapped as `wgpu::Texture`s via wgpu-hal's Metal interop, so a normal
//!   wgpu render pass writes straight into the memory GPUI samples.
//!
//! GPUI 0.2.2 asserts the pixel buffer is `kCVPixelFormatType_420YpCbCr8BiPlanarFullRange`
//! (see `MetalRenderer::draw_surfaces`), so the IOSurface is NV12-shaped.

use std::ffi::c_void;
use std::ptr::NonNull;

use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use core_video::pixel_buffer::{
    CVPixelBuffer, CVPixelBufferRef, kCVPixelBufferIOSurfacePropertiesKey,
    kCVPixelBufferMetalCompatibilityKey, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_io_surface::IOSurfaceRef;
use objc2_metal::{MTLDevice, MTLPixelFormat, MTLTexture, MTLTextureDescriptor, MTLTextureType, MTLTextureUsage};
use vello::wgpu;
use vello::wgpu::hal;

unsafe extern "C" {
    // Get rule: the pixel buffer keeps ownership. (core-video 0.4.3's
    // `CVPixelBuffer::get_io_surface` wraps this under the *create* rule,
    // which over-releases; do not use it.)
    fn CVPixelBufferGetIOSurface(pixel_buffer: CVPixelBufferRef) -> *mut c_void;
}

#[link(name = "IOSurface", kind = "framework")]
unsafe extern "C" {
    fn IOSurfaceIsInUse(surface: *mut c_void) -> u8;
    fn IOSurfaceLock(surface: *mut c_void, options: u32, seed: *mut u32) -> i32;
    fn IOSurfaceUnlock(surface: *mut c_void, options: u32, seed: *mut u32) -> i32;
    fn IOSurfaceGetBaseAddressOfPlane(surface: *mut c_void, plane: usize) -> *mut c_void;
    fn IOSurfaceGetBytesPerRowOfPlane(surface: *mut c_void, plane: usize) -> usize;
}
const K_IOSURFACE_LOCK_READ_ONLY: u32 = 1;

/// One frame's worth of shared memory: the CVPixelBuffer handed to GPUI and
/// the two wgpu textures (luma, chroma) that alias its planes.
pub struct SurfaceSlot {
    pub pixel_buffer: CVPixelBuffer,
    pub y_view: wgpu::TextureView,
    pub cbcr_view: wgpu::TextureView,
    _y: wgpu::Texture,
    _cbcr: wgpu::Texture,
    pub width: u32,
    pub height: u32,
}

impl SurfaceSlot {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        assert!(width.is_multiple_of(2) && height.is_multiple_of(2), "4:2:0 needs even sizes");
        let empty: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[]);
        let options: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[
            (
                unsafe { CFString::wrap_under_get_rule(kCVPixelBufferIOSurfacePropertiesKey) },
                empty.as_CFType(),
            ),
            (
                unsafe { CFString::wrap_under_get_rule(kCVPixelBufferMetalCompatibilityKey) },
                CFBoolean::true_value().as_CFType(),
            ),
        ]);
        let pixel_buffer = CVPixelBuffer::new(
            kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
            width as usize,
            height as usize,
            Some(&options),
        )
        .expect("CVPixelBufferCreate");

        let surface = unsafe { CVPixelBufferGetIOSurface(pixel_buffer.as_concrete_TypeRef()) };
        assert!(!surface.is_null(), "pixel buffer is not IOSurface-backed");
        let surface: &IOSurfaceRef = unsafe { &*(surface as *const IOSurfaceRef) };

        let hal_device = unsafe { device.as_hal::<hal::api::Metal>() }.expect("metal backend");
        let mtl_device = hal_device.raw_device().clone();
        drop(hal_device);

        let plane = |plane: usize, fmt: MTLPixelFormat, wfmt: wgpu::TextureFormat, w: u32, h: u32| {
            let desc = unsafe {
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    fmt, w as usize, h as usize, false,
                )
            };
            desc.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
            let raw: Retained<ProtocolObject<dyn MTLTexture>> = mtl_device
                .newTextureWithDescriptor_iosurface_plane(&desc, surface, plane)
                .expect("newTextureWithDescriptor:iosurface:plane:");
            let hal_tex = unsafe {
                hal::metal::Device::texture_from_raw(
                    raw,
                    wfmt,
                    MTLTextureType::Type2D,
                    1,
                    1,
                    hal::CopyExtent { width: w, height: h, depth: 1 },
                    None,
                )
            };
            let tex = unsafe {
                device.create_texture_from_hal::<hal::api::Metal>(
                    hal_tex,
                    &wgpu::TextureDescriptor {
                        label: Some("iosurface-plane"),
                        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wfmt,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    },
                    wgpu::TextureUses::UNINITIALIZED,
                )
            };
            let view = tex.create_view(&Default::default());
            (tex, view)
        };
        let (y, y_view) = plane(0, MTLPixelFormat::R8Unorm, wgpu::TextureFormat::R8Unorm, width, height);
        let (cbcr, cbcr_view) = plane(
            1,
            MTLPixelFormat::RG8Unorm,
            wgpu::TextureFormat::Rg8Unorm,
            width / 2,
            height / 2,
        );
        SurfaceSlot { pixel_buffer, y_view, cbcr_view, _y: y, _cbcr: cbcr, width, height }
    }

    fn surface(&self) -> *mut c_void {
        unsafe { CVPixelBufferGetIOSurface(self.pixel_buffer.as_concrete_TypeRef()) }
    }

    /// True while some client (e.g. GPUI's CVMetalTexture) still holds the IOSurface.
    pub fn in_use(&self) -> bool {
        unsafe { IOSurfaceIsInUse(self.surface()) != 0 }
    }

    /// Read the IOSurface back on the CPU (lock, decode BT.601 4:2:0 to RGBA):
    /// exactly the bytes GPUI's surface shader samples.
    pub fn decode_rgba(&self) -> Vec<u8> {
        let s = self.surface();
        let (w, h) = (self.width as usize, self.height as usize);
        let mut out = vec![0u8; w * h * 4];
        unsafe {
            let rc = IOSurfaceLock(s, K_IOSURFACE_LOCK_READ_ONLY, std::ptr::null_mut());
            assert_eq!(rc, 0, "IOSurfaceLock");
            let y = NonNull::new(IOSurfaceGetBaseAddressOfPlane(s, 0) as *mut u8).expect("plane 0");
            let uv = NonNull::new(IOSurfaceGetBaseAddressOfPlane(s, 1) as *mut u8).expect("plane 1");
            let ys = IOSurfaceGetBytesPerRowOfPlane(s, 0);
            let uvs = IOSurfaceGetBytesPerRowOfPlane(s, 1);
            for row in 0..h {
                for col in 0..w {
                    let yv = *y.as_ptr().add(row * ys + col) as f32 / 255.0;
                    let o = (row / 2) * uvs + (col / 2) * 2;
                    let cb = *uv.as_ptr().add(o) as f32 / 255.0 - 0.5;
                    let cr = *uv.as_ptr().add(o + 1) as f32 / 255.0 - 0.5;
                    let r = yv + 1.402 * cr;
                    let g = yv - 0.3441 * cb - 0.7141 * cr;
                    let b = yv + 1.772 * cb;
                    let i = (row * w + col) * 4;
                    out[i] = (r.clamp(0.0, 1.0) * 255.0).round() as u8;
                    out[i + 1] = (g.clamp(0.0, 1.0) * 255.0).round() as u8;
                    out[i + 2] = (b.clamp(0.0, 1.0) * 255.0).round() as u8;
                    out[i + 3] = 255;
                }
            }
            IOSurfaceUnlock(s, K_IOSURFACE_LOCK_READ_ONLY, std::ptr::null_mut());
        }
        out
    }
}

/// PSNR (dB) over RGB between two RGBA8 images of the same size.
pub fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let mut se = 0f64;
    let mut n = 0f64;
    for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        for c in 0..3 {
            let d = pa[c] as f64 - pb[c] as f64;
            se += d * d;
            n += 1.0;
        }
    }
    let mse = se / n;
    if mse == 0.0 { f64::INFINITY } else { 10.0 * (255.0f64 * 255.0 / mse).log10() }
}
