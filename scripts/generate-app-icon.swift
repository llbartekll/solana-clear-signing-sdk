import AppKit
import ImageIO
import UniformTypeIdentifiers
let size = 1024
let context = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
let colors = [CGColor(red: 0.035, green: 0.07, blue: 0.17, alpha: 1), CGColor(red: 0.10, green: 0.20, blue: 0.36, alpha: 1)] as CFArray
let gradient = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: colors, locations: [0, 1])!
context.drawLinearGradient(gradient, start: CGPoint(x: 0, y: 0), end: CGPoint(x: 1024, y: 1024), options: [])
// An open document with readable lines and an approval mark.
context.setFillColor(CGColor(red: 0.94, green: 0.97, blue: 1, alpha: 1))
context.addPath(CGPath(roundedRect: CGRect(x: 260, y: 205, width: 504, height: 614), cornerWidth: 68, cornerHeight: 68, transform: nil))
context.fillPath()
context.setStrokeColor(CGColor(red: 0.38, green: 0.49, blue: 0.62, alpha: 1))
context.setLineWidth(32); context.setLineCap(.round)
for (y, end): (CGFloat, CGFloat) in [(690, 658), (606, 576)] {
 context.move(to: CGPoint(x: 356, y: y)); context.addLine(to: CGPoint(x: end, y: y)); context.strokePath()
}
context.setStrokeColor(CGColor(red: 0.03, green: 0.59, blue: 0.48, alpha: 1))
context.setLineWidth(60); context.setLineJoin(.round)
context.move(to: CGPoint(x: 363, y: 430)); context.addLine(to: CGPoint(x: 462, y: 335)); context.addLine(to: CGPoint(x: 663, y: 516)); context.strokePath()
let url = URL(fileURLWithPath: CommandLine.arguments[1])
let output = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)!
CGImageDestinationAddImage(output, context.makeImage()!, nil)
precondition(CGImageDestinationFinalize(output))
