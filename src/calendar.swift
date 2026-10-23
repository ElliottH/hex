import EventKit
import Foundation

/// One shared store: EventKit expects the store that asked for access to read
/// events, and its change notifications are what the reminder poll watches.
private let store = EKEventStore()
private let changeLock = NSLock()
/// Serializes store reads: Settings and the reminder poll read from
/// different threads.
private let storeLock = NSLock()
private var changes: UInt64 = 0
private var observer: NSObjectProtocol?

/// Notes beyond this are cut: call links sit near the top of invitations,
/// and Rust parses the text on every poll.
private let notesLimit = 16_384

private struct BridgeCalendar: Codable {
    let id: String
    let title: String
    let source: String
    let color: String
}

private struct BridgeEvent: Codable {
    let id: String
    let calendar: String
    let title: String
    let startMs: Double
    let endMs: Double
    let allDay: Bool
    let free: Bool
    let cancelled: Bool
    let declined: Bool
    let attendees: [String]
    let location: String?
    let notes: String?
    let url: String?
}

private func statusCode(_ status: EKAuthorizationStatus) -> Int32 {
    switch status {
    case .notDetermined: return 0
    case .restricted: return 1
    case .denied: return 2
    case .fullAccess: return 3
    case .writeOnly: return 4
    @unknown default: return 2
    }
}

private func json<T: Encodable>(_ value: T) -> UnsafeMutablePointer<CChar>? {
    guard let data = try? JSONEncoder().encode(value) else { return nil }
    return strdup(String(decoding: data, as: UTF8.self))
}

private func hexColor(_ color: CGColor?) -> String {
    guard let rgb = color?.converted(to: CGColorSpace(name: CGColorSpace.sRGB)!, intent: .defaultIntent, options: nil),
          let components = rgb.components, components.count >= 3 else {
        return ""
    }
    let byte = { (value: CGFloat) in Int((min(max(value, 0), 1) * 255).rounded()) }
    return String(format: "#%02X%02X%02X", byte(components[0]), byte(components[1]), byte(components[2]))
}

private func startObserving() {
    changeLock.lock()
    defer { changeLock.unlock() }
    guard observer == nil else { return }
    observer = NotificationCenter.default.addObserver(
        forName: .EKEventStoreChanged,
        object: store,
        queue: nil
    ) { _ in
        changeLock.lock()
        changes &+= 1
        changeLock.unlock()
    }
}

@_cdecl("hex_calendar_authorization")
public func hexCalendarAuthorization() -> Int32 {
    statusCode(EKEventStore.authorizationStatus(for: .event))
}

/// Shows the system prompt when access was never decided; blocks the calling
/// (background) thread until the user answers. Returns the resulting status.
@_cdecl("hex_calendar_request_access")
public func hexCalendarRequestAccess() -> Int32 {
    let semaphore = DispatchSemaphore(value: 0)
    store.requestFullAccessToEvents { _, _ in
        semaphore.signal()
    }
    semaphore.wait()
    let status = EKEventStore.authorizationStatus(for: .event)
    if status == .fullAccess {
        storeLock.lock()
        store.reset()
        storeLock.unlock()
        startObserving()
    }
    return statusCode(status)
}

/// Counts EventKit store changes since the first call with access.
@_cdecl("hex_calendar_change_count")
public func hexCalendarChangeCount() -> UInt64 {
    guard EKEventStore.authorizationStatus(for: .event) == .fullAccess else { return 0 }
    startObserving()
    changeLock.lock()
    defer { changeLock.unlock() }
    return changes
}

@_cdecl("hex_calendar_calendars")
public func hexCalendarCalendars() -> UnsafeMutablePointer<CChar>? {
    guard EKEventStore.authorizationStatus(for: .event) == .fullAccess else { return nil }
    storeLock.lock()
    defer { storeLock.unlock() }
    let calendars = store.calendars(for: .event).map {
        BridgeCalendar(
            id: $0.calendarIdentifier,
            title: $0.title,
            source: $0.source?.title ?? "",
            color: hexColor($0.cgColor)
        )
    }
    return json(calendars)
}

/// Events overlapping the window, soonest first, at most `limit`.
@_cdecl("hex_calendar_events")
public func hexCalendarEvents(_ fromMs: Double, _ toMs: Double, _ limit: Int32) -> UnsafeMutablePointer<CChar>? {
    guard EKEventStore.authorizationStatus(for: .event) == .fullAccess else { return nil }
    storeLock.lock()
    defer { storeLock.unlock() }
    let predicate = store.predicateForEvents(
        withStart: Date(timeIntervalSince1970: fromMs / 1000),
        end: Date(timeIntervalSince1970: toMs / 1000),
        calendars: nil
    )
    let events = store.events(matching: predicate)
        .sorted { $0.startDate < $1.startDate }
        .prefix(Int(max(limit, 0)))
        .map { event -> BridgeEvent in
            let attendees = event.attendees ?? []
            let declined = attendees.contains { $0.isCurrentUser && $0.participantStatus == .declined }
            let others = attendees
                .filter { !$0.isCurrentUser && $0.participantType != .room && $0.participantType != .resource }
                .map { $0.name ?? $0.url.absoluteString.replacingOccurrences(of: "mailto:", with: "") }
            return BridgeEvent(
                id: event.eventIdentifier ?? event.calendarItemIdentifier,
                calendar: event.calendar?.calendarIdentifier ?? "",
                title: event.title ?? "",
                startMs: event.startDate.timeIntervalSince1970 * 1000,
                endMs: event.endDate.timeIntervalSince1970 * 1000,
                allDay: event.isAllDay,
                free: event.availability == .free,
                cancelled: event.status == .canceled,
                declined: declined,
                attendees: others,
                location: event.location,
                notes: event.notes.map { String($0.prefix(notesLimit)) },
                url: event.url?.absoluteString
            )
        }
    return json(Array(events))
}

@_cdecl("hex_calendar_free_string")
public func hexCalendarFreeString(_ string: UnsafeMutablePointer<CChar>?) {
    free(string)
}
