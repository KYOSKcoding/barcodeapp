import SwiftUI

/// Scanner state machine — mirrors Android's `BarcodeState` enum from the shared KMP module.
enum ScannerState {
    case idle
    case scanningTicket
    case connecting
    case ready
    case scanningCode
    case scanned(code: String, kind: Int32, numbers: [String], shops: [ShopInfo], imageJpeg: Data)
    case sending
}

// MARK: – Root view

struct ContentView: View {
    @StateObject private var bridge = IrohBridge()
    @State private var scannerState: ScannerState = .idle
    @State private var statusMessage: String = ""
    @State private var showingTicketScanner = false
    @State private var showingCodeScanner = false
    @State private var isLocalMode = false
    @State private var sendError: String? = nil

    var body: some View {
        NavigationStack {
            VStack(spacing: 20) {
                statusLabel
                scannedInfo
                buttons
            }
            .padding(32)
            .navigationTitle("Barcode Scanner")
            .sheet(isPresented: $showingTicketScanner) {
                BarcodeScannerView { code, _, _ in
                    showingTicketScanner = false
                    onTicketScanned(ticket: code)
                }
                .ignoresSafeArea()
            }
            .sheet(isPresented: $showingCodeScanner) {
                BarcodeScannerView { code, kind, jpeg in
                    showingCodeScanner = false
                    onCodeScanned(code: code, kind: kind, imageJpeg: jpeg)
                }
                .ignoresSafeArea()
            }
        }
    }

    // MARK: – Sub-views

    @ViewBuilder
    private var statusLabel: some View {
        switch scannerState {
        case .idle:
            Text("Ready to connect").font(.title2)
        case .scanningTicket:
            Text("Scanning ticket…").font(.title2)
        case .connecting:
            ProgressView("Connecting…")
        case .ready:
            Text("Connected ✓").font(.title2).foregroundColor(.green)
        case .scanningCode:
            Text("Scanning code…").font(.title2)
        case .scanned:
            Text("Scanned").font(.title2)
        case .sending:
            ProgressView(statusMessage.isEmpty ? "Sending…" : statusMessage)
        }
    }

    @ViewBuilder
    private var scannedInfo: some View {
        if case let .scanned(code, _, numbers, detectedShops, _) = scannerState {
            VStack(alignment: .leading, spacing: 8) {
                // Extracted card number(s)
                if !numbers.isEmpty {
                    ForEach(numbers, id: \.self) { number in
                        HStack {
                            Text(number)
                                .font(.system(.body, design: .monospaced))
                                .textSelection(.enabled)
                            Spacer()
                            Button {
                                UIPasteboard.general.string = number
                            } label: {
                                Label("Copy", systemImage: "doc.on.doc")
                                    .labelStyle(.iconOnly)
                            }
                        }
                        .padding(8)
                        .background(Color(.systemGray6))
                        .cornerRadius(8)
                    }
                }

                // Raw code (greyed, only if different from extracted)
                let singleMatchesRaw = numbers.count == 1 && numbers[0] == code.filter(\.isNumber)
                if !singleMatchesRaw && !code.isEmpty {
                    Text(code)
                        .font(.caption)
                        .foregroundColor(.secondary)
                        .textSelection(.enabled)
                        .lineLimit(3)
                }

                // Shop buttons
                if !detectedShops.isEmpty {
                    HStack {
                        ForEach(detectedShops, id: \.name) { shop in
                            Link(shop.name, destination: shop.url)
                                .buttonStyle(.bordered)
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private var buttons: some View {
        VStack(spacing: 12) {
            switch scannerState {
            case .idle:
                Button("Connect to Receiver") {
                    showingTicketScanner = true
                    scannerState = .scanningTicket
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)

                Button("Scan on Phone") {
                    isLocalMode = true
                    showingCodeScanner = true
                    scannerState = .scanningCode
                }
                .buttonStyle(.bordered)
                .controlSize(.large)

                if bridge.isConnected {
                    Button("Resume Scanning") {
                        scannerState = .ready
                    }
                    .buttonStyle(.bordered)
                }

            case .ready:
                Button("Scan") {
                    showingCodeScanner = true
                    scannerState = .scanningCode
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)

                Button("Disconnect") { onDisconnect() }
                    .buttonStyle(.bordered)
                    .tint(.red)

            case .scanned(let code, let kind, _, _, let jpeg):
                if isLocalMode {
                    Button("Scan Again") {
                        showingCodeScanner = true
                        scannerState = .scanningCode
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                } else {
                    Button("Send") {
                        Task { await sendScan(code: code, kind: kind, imageJpeg: jpeg) }
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                }

                Button("Back") { onBack() }
                    .buttonStyle(.bordered)

            case .sending:
                EmptyView()

            default:
                EmptyView()
            }

            if let err = sendError {
                Text(err)
                    .foregroundColor(.red)
                    .font(.callout)
            }
        }
    }

    // MARK: – State transitions

    private func onTicketScanned(ticket: String) {
        scannerState = .connecting
        Task {
            let ok = await bridge.connect(ticket: ticket)
            if ok {
                scannerState = .ready
            } else {
                scannerState = .idle
                sendError = "Connection failed"
            }
        }
    }

    private func onCodeScanned(code: String, kind: Int32, imageJpeg: Data) {
        let numbers = extractCardNumbers(code: code)
        let detectedShops = detectShops(code: code)
        scannerState = .scanned(code: code, kind: kind, numbers: numbers, shops: detectedShops, imageJpeg: imageJpeg)
    }

    private func sendScan(code: String, kind: Int32, imageJpeg: Data) async {
        sendError = nil
        scannerState = .sending
        statusMessage = "Sending…"

        let ok = await bridge.sendScan(kind: kind, code: code, imageJpeg: imageJpeg)
        if ok {
            scannerState = .ready
        } else {
            sendError = "Send failed — reconnect required"
            bridge.disconnect()
            scannerState = .idle
        }
    }

    private func onBack() {
        isLocalMode = false
        sendError = nil
        scannerState = bridge.isConnected ? .ready : .idle
    }

    private func onDisconnect() {
        bridge.disconnect()
        isLocalMode = false
        sendError = nil
        scannerState = .idle
    }
}

#Preview {
    ContentView()
}
