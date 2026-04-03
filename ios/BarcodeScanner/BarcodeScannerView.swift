import AVFoundation
import SwiftUI
import UIKit

/// AVFoundation-based barcode/QR code scanner.
///
/// Wraps `AVCaptureSession` in a `UIViewControllerRepresentable` for use in SwiftUI.
/// Reports the first decoded code string and (optionally) a JPEG image of the captured frame.
struct BarcodeScannerView: UIViewControllerRepresentable {

    /// Called when a code is successfully decoded.
    /// - Parameters:
    ///   - code: The decoded string.
    ///   - kind: 0 = barcode, 1 = QR code.
    ///   - imageJpeg: JPEG-compressed frame from the capture session (may be empty).
    var onScan: (String, Int32, Data) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onScan: onScan)
    }

    func makeUIViewController(context: Context) -> ScannerViewController {
        let vc = ScannerViewController()
        vc.coordinator = context.coordinator
        return vc
    }

    func updateUIViewController(_ uiViewController: ScannerViewController, context: Context) {}
}

// MARK: – ScannerViewController

final class ScannerViewController: UIViewController {
    var coordinator: BarcodeScannerView.Coordinator?

    private var captureSession: AVCaptureSession?
    private var previewLayer: AVCaptureVideoPreviewLayer?
    private var photoOutput: AVCapturePhotoOutput?
    private var pendingCode: String?
    private var pendingKind: Int32 = 0

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        requestCameraAccess()
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        DispatchQueue.global(qos: .userInitiated).async {
            self.captureSession?.startRunning()
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        captureSession?.stopRunning()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        previewLayer?.frame = view.layer.bounds
    }

    private func requestCameraAccess() {
        AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
            DispatchQueue.main.async {
                if granted {
                    self?.setupCaptureSession()
                }
            }
        }
    }

    private func setupCaptureSession() {
        let session = AVCaptureSession()
        session.beginConfiguration()
        session.sessionPreset = .high

        guard
            let device = AVCaptureDevice.default(for: .video),
            let input = try? AVCaptureDeviceInput(device: device),
            session.canAddInput(input)
        else { return }
        session.addInput(input)

        // Metadata output for barcode/QR decoding
        let metaOutput = AVCaptureMetadataOutput()
        guard session.canAddOutput(metaOutput) else { return }
        session.addOutput(metaOutput)
        metaOutput.setMetadataObjectsDelegate(self, queue: .main)
        metaOutput.metadataObjectTypes = [
            .qr,
            .ean8, .ean13, .pdf417,
            .code128, .code39, .code93,
            .upce, .dataMatrix, .aztec,
            .itf14,
        ]

        // Photo output for capturing a JPEG frame at the moment of scan
        let photo = AVCapturePhotoOutput()
        if session.canAddOutput(photo) {
            session.addOutput(photo)
            photoOutput = photo
        }

        session.commitConfiguration()

        let preview = AVCaptureVideoPreviewLayer(session: session)
        preview.videoGravity = .resizeAspectFill
        preview.frame = view.layer.bounds
        view.layer.addSublayer(preview)
        previewLayer = preview

        captureSession = session
        DispatchQueue.global(qos: .userInitiated).async {
            session.startRunning()
        }
    }
}

// MARK: – AVCaptureMetadataOutputObjectsDelegate

extension ScannerViewController: AVCaptureMetadataOutputObjectsDelegate {
    func metadataOutput(
        _ output: AVCaptureMetadataOutput,
        didOutput metadataObjects: [AVMetadataObject],
        from connection: AVCaptureConnection
    ) {
        guard
            coordinator?.hasReported == false,
            let meta = metadataObjects.first as? AVMetadataMachineReadableCodeObject,
            let code = meta.stringValue
        else { return }

        coordinator?.hasReported = true
        captureSession?.stopRunning()

        let kind: Int32 = meta.type == .qr ? 1 : 0
        pendingCode = code
        pendingKind = kind

        // Capture a still frame for the JPEG image
        if let photoOutput {
            let settings = AVCapturePhotoSettings()
            photoOutput.capturePhoto(with: settings, delegate: self)
        } else {
            coordinator?.onScan(code, kind, Data())
        }
    }
}

// MARK: – AVCapturePhotoCaptureDelegate

extension ScannerViewController: AVCapturePhotoCaptureDelegate {
    func photoOutput(
        _ output: AVCapturePhotoOutput,
        didFinishProcessingPhoto photo: AVCapturePhoto,
        error: Error?
    ) {
        let code = pendingCode ?? ""
        let kind = pendingKind

        var jpeg = Data()
        if let fileData = photo.fileDataRepresentation(),
           let image = UIImage(data: fileData) {
            // Scale to max 1080px on the long side, matching Android behaviour
            jpeg = scaleAndCompress(image: image, maxDimension: 1080, quality: 0.85)
        }

        coordinator?.onScan(code, kind, jpeg)
    }
}

// MARK: – Coordinator

extension BarcodeScannerView {
    final class Coordinator {
        let onScan: (String, Int32, Data) -> Void
        var hasReported = false

        init(onScan: @escaping (String, Int32, Data) -> Void) {
            self.onScan = onScan
        }
    }
}

// MARK: – Image helpers

private func scaleAndCompress(image: UIImage, maxDimension: CGFloat, quality: CGFloat) -> Data {
    let size = image.size
    let longSide = max(size.width, size.height)
    guard longSide > maxDimension else {
        return image.jpegData(compressionQuality: quality) ?? Data()
    }

    let scale = maxDimension / longSide
    let newSize = CGSize(width: size.width * scale, height: size.height * scale)

    let renderer = UIGraphicsImageRenderer(size: newSize)
    let scaled = renderer.image { _ in
        image.draw(in: CGRect(origin: .zero, size: newSize))
    }
    return scaled.jpegData(compressionQuality: quality) ?? Data()
}
