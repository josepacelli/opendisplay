import XCTest

final class VirtualCanvasSizingTests: XCTestCase {
    func testSmallCanvasStartsAtRequestedSize() {
        let requested = VirtualCanvasSizing.requested(pixelsWide: 2_388, pixelsHigh: 1_668)!
        XCTAssertEqual(VirtualCanvasSizing.bootstrap(for: requested), requested)
    }

    func testLargeLandscapeCanvasBootstrapsAtSameAspect() {
        let requested = VirtualCanvasSizing.requested(pixelsWide: 4_096, pixelsHigh: 2_304)!
        XCTAssertEqual(VirtualCanvasSizing.bootstrap(for: requested),
                       VirtualCanvasSize(pointsWide: 1_600, pointsHigh: 900))
    }

    func testLargePortraitCanvasBootstrapsWithRotatedEnvelope() {
        let requested = VirtualCanvasSizing.requested(pixelsWide: 2_304, pixelsHigh: 4_096)!
        XCTAssertEqual(VirtualCanvasSizing.bootstrap(for: requested),
                       VirtualCanvasSize(pointsWide: 900, pointsHigh: 1_600))
    }

    func testInvalidCanvasIsRejected() {
        XCTAssertNil(VirtualCanvasSizing.requested(pixelsWide: 1, pixelsHigh: 1_080))
    }

    func testPlanReservesRequestedCapacityBeyondDefaultHeadroom() {
        let plan = VirtualCanvasSizing.plan(pixelsWide: 10_240, pixelsHigh: 4_320)!

        XCTAssertEqual(plan.bootstrap,
                       VirtualCanvasSize(pointsWide: 1_600, pointsHigh: 674))
        XCTAssertEqual(plan.descriptorMaxPixelsPerAxis, 10_240)
    }
}
