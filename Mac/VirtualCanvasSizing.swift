import Foundation

struct VirtualCanvasSize: Equatable {
    let pointsWide: Int
    let pointsHigh: Int

    var pixelsWide: Int { pointsWide * 2 }
    var pixelsHigh: Int { pointsHigh * 2 }
    var cgSize: CGSize { CGSize(width: pointsWide, height: pointsHigh) }
}

struct VirtualCanvasPlan: Equatable {
    let requested: VirtualCanvasSize
    let bootstrap: VirtualCanvasSize
    let descriptorMaxPixelsPerAxis: Int
}

/// WindowServer can refuse a large CGVirtualDisplay when that mode is present
/// at creation, while accepting the same mode when it is applied to an online
/// display. This is a local macOS startup workaround, not a protocol limit:
/// start within a conservative 3200x1800 pixel envelope and promote the same
/// identity after ScreenCaptureKit sees it.
enum VirtualCanvasSizing {
    private static let bootstrapLongEdgePoints = 1_600
    private static let bootstrapShortEdgePoints = 900
    private static let reservedPixelsPerAxis = 8_192

    static func plan(pixelsWide: Int, pixelsHigh: Int) -> VirtualCanvasPlan? {
        guard let requested = requested(pixelsWide: pixelsWide,
                                        pixelsHigh: pixelsHigh) else { return nil }
        return VirtualCanvasPlan(
            requested: requested,
            bootstrap: bootstrap(for: requested),
            descriptorMaxPixelsPerAxis: max(reservedPixelsPerAxis,
                                             requested.pixelsWide,
                                             requested.pixelsHigh))
    }

    static func requested(pixelsWide: Int, pixelsHigh: Int) -> VirtualCanvasSize? {
        guard pixelsWide >= 4, pixelsHigh >= 4 else { return nil }
        let width = (pixelsWide / 2) & ~1
        let height = (pixelsHigh / 2) & ~1
        guard width >= 2, height >= 2 else { return nil }
        return VirtualCanvasSize(pointsWide: width, pointsHigh: height)
    }

    static func bootstrap(for requested: VirtualCanvasSize) -> VirtualCanvasSize {
        let landscape = requested.pointsWide >= requested.pointsHigh
        let maximumWidth = landscape ? bootstrapLongEdgePoints : bootstrapShortEdgePoints
        let maximumHeight = landscape ? bootstrapShortEdgePoints : bootstrapLongEdgePoints
        let scale = min(1, min(Double(maximumWidth) / Double(requested.pointsWide),
                               Double(maximumHeight) / Double(requested.pointsHigh)))
        guard scale < 1 else { return requested }
        return VirtualCanvasSize(
            pointsWide: max(2, Int(Double(requested.pointsWide) * scale) & ~1),
            pointsHigh: max(2, Int(Double(requested.pointsHigh) * scale) & ~1))
    }
}
