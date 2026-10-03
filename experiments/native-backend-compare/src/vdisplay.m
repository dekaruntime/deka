// A CoreGraphics virtual display for the measurements: windows on it are
// composited and presented by the window server like on any display, but no
// panel shows them. Private API (CGVirtualDisplay, macOS 11+); the interfaces
// below are the ones Chromium's test utility declares
// (ui/display/mac/test/virtual_display_util_mac.mm), which this follows.
// Classes are looked up at run time, so nothing links against private symbols.
#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>
#include <dispatch/dispatch.h>
#include <stdio.h>
#include <unistd.h>

@interface CGVirtualDisplayDescriptor : NSObject
@property(nonatomic) unsigned int vendorID;
@property(nonatomic) unsigned int productID;
@property(nonatomic) unsigned int serialNum;
@property(strong, nonatomic) NSString *name;
@property(nonatomic) struct CGSize sizeInMillimeters;
@property(nonatomic) unsigned int maxPixelsWide;
@property(nonatomic) unsigned int maxPixelsHigh;
@property(nonatomic) struct CGPoint redPrimary;
@property(nonatomic) struct CGPoint greenPrimary;
@property(nonatomic) struct CGPoint bluePrimary;
@property(nonatomic) struct CGPoint whitePoint;
@property(strong, nonatomic) id queue;
@property(copy, nonatomic) id terminationHandler;
@property(nonatomic) unsigned int serialNumber;
@end

@interface CGVirtualDisplayMode : NSObject
- (id)initWithWidth:(unsigned int)width height:(unsigned int)height refreshRate:(double)refreshRate;
@end

@interface CGVirtualDisplaySettings : NSObject
@property(strong, nonatomic) NSArray *modes;
@property(nonatomic) unsigned int hiDPI;
@property(nonatomic) unsigned int rotation;
@end

@interface CGVirtualDisplay : NSObject
@property(readonly, nonatomic) unsigned int displayID;
- (id)initWithDescriptor:(id)descriptor;
- (BOOL)applySettings:(id)settings;
@end

static CGVirtualDisplay *g_display;

static int is_online(CGDirectDisplayID id) {
  CGDirectDisplayID ids[32];
  uint32_t n = 0;
  if (CGGetOnlineDisplayList(32, ids, &n) != kCGErrorSuccess) return 0;
  for (uint32_t i = 0; i < n; i++)
    if (ids[i] == id) return 1;
  return 0;
}

// Pick the mode that is `width_pt` x `height_pt` points at `scale` pixels per
// point, and switch to it if the display is not already there.
static int in_mode(CGDirectDisplayID id, uint32_t width_pt, uint32_t height_pt, uint32_t scale) {
  CGDisplayModeRef current = CGDisplayCopyDisplayMode(id);
  int ok = current && CGDisplayModeGetWidth(current) == width_pt &&
           CGDisplayModeGetHeight(current) == height_pt &&
           CGDisplayModeGetPixelWidth(current) == width_pt * scale;
  CGDisplayModeRelease(current);
  CGRect b = CGDisplayBounds(id);
  return ok && b.size.width == width_pt && b.size.height == height_pt;
}

static int ensure_mode(CGDirectDisplayID id, uint32_t width_pt, uint32_t height_pt, uint32_t scale) {
  // The display comes online before its first mode is applied: give it time.
  for (int i = 0; i < 200; i++) {
    if (in_mode(id, width_pt, height_pt, scale)) return 1;
    usleep(10000);
  }
  NSDictionary *options = @{(__bridge NSString *)kCGDisplayShowDuplicateLowResolutionModes : @YES};
  CFArrayRef modes = CGDisplayCopyAllDisplayModes(id, (__bridge CFDictionaryRef)options);
  if (!modes) return 0;
  CGDisplayModeRef wanted = NULL;
  for (CFIndex i = 0; i < CFArrayGetCount(modes); i++) {
    CGDisplayModeRef m = (CGDisplayModeRef)CFArrayGetValueAtIndex(modes, i);
    if (CGDisplayModeGetWidth(m) == width_pt && CGDisplayModeGetHeight(m) == height_pt &&
        CGDisplayModeGetPixelWidth(m) == width_pt * scale) {
      wanted = m;
      break;
    }
  }
  int set = wanted && CGDisplaySetDisplayMode(id, wanted, NULL) == kCGErrorSuccess;
  CFRelease(modes);
  for (int i = 0; set && i < 500; i++) {
    if (in_mode(id, width_pt, height_pt, scale)) return 1;
    usleep(10000);
  }
  return 0;
}

// Create the display and wait until it is online in the requested mode.
// Returns its CGDirectDisplayID, or 0 (with a message on stderr).
uint32_t cmp_vd_create(uint32_t width_pt, uint32_t height_pt, uint32_t scale, double refresh,
                       double ppi, uint32_t vendor, uint32_t product, uint32_t serial,
                       const char *name) {
  @autoreleasepool {
    if (g_display) return g_display.displayID;
    Class descriptor_class = NSClassFromString(@"CGVirtualDisplayDescriptor");
    Class display_class = NSClassFromString(@"CGVirtualDisplay");
    Class mode_class = NSClassFromString(@"CGVirtualDisplayMode");
    Class settings_class = NSClassFromString(@"CGVirtualDisplaySettings");
    if (!descriptor_class || !display_class || !mode_class || !settings_class) {
      fprintf(stderr, "vdisplay: CGVirtualDisplay classes not found (macOS 11+ needed)\n");
      return 0;
    }
    uint32_t width_px = width_pt * scale, height_px = height_pt * scale;
    CGVirtualDisplayDescriptor *descriptor = [[descriptor_class alloc] init];
    descriptor.queue = dispatch_get_global_queue(DISPATCH_QUEUE_PRIORITY_HIGH, 0);
    descriptor.name = [NSString stringWithUTF8String:name];
    // sRGB primaries and D65, as Chromium sets them.
    descriptor.whitePoint = CGPointMake(0.3125, 0.3291);
    descriptor.bluePrimary = CGPointMake(0.1494, 0.0557);
    descriptor.greenPrimary = CGPointMake(0.2559, 0.6983);
    descriptor.redPrimary = CGPointMake(0.6797, 0.3203);
    descriptor.maxPixelsWide = width_px;
    descriptor.maxPixelsHigh = height_px;
    descriptor.sizeInMillimeters = CGSizeMake(25.4 * width_px / ppi, 25.4 * height_px / ppi);
    // macOS 14+ wants a non-zero vendor and distinct serial numbers.
    descriptor.vendorID = vendor;
    descriptor.productID = product;
    descriptor.serialNum = serial;
    descriptor.serialNumber = serial;
    descriptor.terminationHandler = nil;
    CGVirtualDisplay *display = [[display_class alloc] initWithDescriptor:descriptor];
    if (!display) {
      fprintf(stderr, "vdisplay: initWithDescriptor failed\n");
      return 0;
    }
    CGVirtualDisplaySettings *settings = [[settings_class alloc] init];
    settings.hiDPI = scale > 1;
    settings.rotation = 0;
    // A mode is given in points; with hiDPI its backing store is `scale`x.
    settings.modes = @[ [[mode_class alloc] initWithWidth:width_pt height:height_pt refreshRate:refresh] ];
    if (![display applySettings:settings]) {
      fprintf(stderr, "vdisplay: applySettings failed\n");
      return 0;
    }
    g_display = display;
    CGDirectDisplayID id = display.displayID;
    for (int i = 0; i < 500 && !is_online(id); i++) usleep(10000);
    if (!is_online(id)) {
      fprintf(stderr, "vdisplay: display %u never came online\n", id);
      g_display = nil;
      return 0;
    }
    if (!ensure_mode(id, width_pt, height_pt, scale)) {
      fprintf(stderr, "vdisplay: no %ux%u@%ux mode on display %u\n", width_pt, height_pt, scale, id);
      g_display = nil;
      return 0;
    }
    return id;
  }
}

// Remove the display (releasing the object removes it) and wait until it is
// gone. Returns milliseconds waited, or -1 if it is still online after 5 s.
int cmp_vd_destroy(void) {
  CGDirectDisplayID id;
  @autoreleasepool {
    if (!g_display) return 0;
    id = g_display.displayID;
    g_display = nil;
  }
  for (int i = 0; i < 500; i++) {
    if (!is_online(id)) return i * 10;
    usleep(10000);
  }
  return -1;
}

