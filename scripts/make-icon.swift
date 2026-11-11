// Renders The Writer's app icon with CoreGraphics.
//
// Usage: swift scripts/make-icon.swift <output.iconset directory>
// (scripts/make-icon.sh wraps this and runs iconutil.)
//
// The mark: a sheet of warm paper, a few lines of ink, and the terracotta caret
// waiting at the end of the last line. Flat colours from the Paper theme, no text.
// Sizes of 32 pixels and below use a simplified drawing (two lines, heavier
// strokes) so the caret still reads at 16px.

import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

func color(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(
        srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
        green: CGFloat((hex >> 8) & 0xFF) / 255,
        blue: CGFloat(hex & 0xFF) / 255,
        alpha: alpha)
}

let paper = color(0xFAF8F4)
let ink = color(0x1F1C18)
let accent = color(0xB0503A)

/// A horizontal bar with fully rounded ends, in the 1024-unit design space.
func bar(_ ctx: CGContext, x0: CGFloat, x1: CGFloat, y: CGFloat, thickness: CGFloat, _ fill: CGColor) {
    let rect = CGRect(x: x0, y: y - thickness / 2, width: x1 - x0, height: thickness)
    ctx.addPath(CGPath(roundedRect: rect, cornerWidth: thickness / 2, cornerHeight: thickness / 2, transform: nil))
    ctx.setFillColor(fill)
    ctx.fillPath()
}

/// The caret: a vertical bar with slightly rounded ends.
func caret(_ ctx: CGContext, x: CGFloat, y0: CGFloat, y1: CGFloat, width: CGFloat) {
    let rect = CGRect(x: x - width / 2, y: y0, width: width, height: y1 - y0)
    let r = width * 0.3
    ctx.addPath(CGPath(roundedRect: rect, cornerWidth: r, cornerHeight: r, transform: nil))
    ctx.setFillColor(accent)
    ctx.fillPath()
}

func render(pixels: Int) -> CGImage {
    let ctx = CGContext(
        data: nil, width: pixels, height: pixels, bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpace(name: CGColorSpace.sRGB)!,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.interpolationQuality = .high
    ctx.setShouldAntialias(true)
    // Design space: 1024 units, origin top-left, y pointing down.
    let s = CGFloat(pixels) / 1024
    ctx.translateBy(x: 0, y: CGFloat(pixels))
    ctx.scaleBy(x: s, y: -s)

    let small = pixels <= 32

    // The sheet: the standard macOS icon body (824 of 1024 units, radius 185).
    let body = CGRect(x: 100, y: 100, width: 824, height: 824)
    let bodyPath = CGPath(roundedRect: body, cornerWidth: 185, cornerHeight: 185, transform: nil)

    ctx.saveGState()
    if !small {
        // Shadow offsets and blur are in device pixels; the offset's y is not flipped
        // by the CTM, so a negative value drops the shadow downwards.
        ctx.setShadow(offset: CGSize(width: 0, height: -12 * s), blur: 28 * s, color: color(0x1F1C18, 0.30))
    }
    ctx.addPath(bodyPath)
    ctx.setFillColor(paper)
    ctx.fillPath()
    ctx.restoreGState()

    // Hairline edge so the sheet holds on a light background.
    ctx.addPath(bodyPath)
    ctx.setStrokeColor(color(0x1F1C18, small ? 0.38 : 0.16))
    ctx.setLineWidth(small ? 40 : 6)
    ctx.saveGState()
    ctx.addPath(bodyPath)
    ctx.clip()
    ctx.addPath(bodyPath)
    ctx.strokePath()
    ctx.restoreGState()

    if small {
        // Two lines and a heavy caret.
        bar(ctx, x0: 250, x1: 774, y: 380, thickness: 100, ink)
        bar(ctx, x0: 250, x1: 510, y: 640, thickness: 100, ink)
        caret(ctx, x: 650, y0: 520, y1: 770, width: 120)
    } else {
        // Three lines; the last one is short and ends at the caret.
        bar(ctx, x0: 262, x1: 762, y: 330, thickness: 60, ink)
        bar(ctx, x0: 262, x1: 690, y: 490, thickness: 60, ink)
        bar(ctx, x0: 262, x1: 462, y: 660, thickness: 60, ink)
        caret(ctx, x: 548, y0: 578, y1: 742, width: 52)
    }
    return ctx.makeImage()!
}

func writePNG(_ image: CGImage, to url: URL) {
    let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)!
    CGImageDestinationAddImage(dest, image, nil)
    guard CGImageDestinationFinalize(dest) else {
        FileHandle.standardError.write("make-icon: could not write \(url.path)\n".data(using: .utf8)!)
        exit(1)
    }
}

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write("usage: make-icon.swift <output.iconset>\n".data(using: .utf8)!)
    exit(2)
}
let out = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)

for points in [16, 32, 128, 256, 512] {
    writePNG(render(pixels: points), to: out.appendingPathComponent("icon_\(points)x\(points).png"))
    writePNG(render(pixels: points * 2), to: out.appendingPathComponent("icon_\(points)x\(points)@2x.png"))
}
print("make-icon: wrote 10 PNGs to \(out.path)")
