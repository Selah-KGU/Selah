import Darwin
import Foundation
import FoundationModels

private let cancelledMessage = "推論はキャンセルされました"

private final class TaskRegistry: @unchecked Sendable {
    private let lock = NSLock()
    private var tasks: [String: Task<Void, Never>] = [:]
    private var cancelled: Set<String> = []

    func insert(_ id: String, _ task: Task<Void, Never>) {
        guard !id.isEmpty else { return }
        lock.lock()
        tasks[id] = task
        lock.unlock()
    }

    func remove(_ id: String) {
        guard !id.isEmpty else { return }
        lock.lock()
        tasks.removeValue(forKey: id)
        lock.unlock()
    }

    func cancel(_ id: String) {
        guard !id.isEmpty else { return }
        lock.lock()
        cancelled.insert(id)
        let task = tasks[id]
        lock.unlock()
        task?.cancel()
    }

    func clear(_ id: String) {
        guard !id.isEmpty else { return }
        lock.lock()
        cancelled.remove(id)
        lock.unlock()
    }

    func isCancelled(_ id: String) -> Bool {
        guard !id.isEmpty else { return false }
        lock.lock()
        defer { lock.unlock() }
        return cancelled.contains(id)
    }
}

private let registry = TaskRegistry()

private struct SendableCallback: @unchecked Sendable {
    let callback: (@convention(c) (UnsafePointer<CChar>?, Int32, UnsafeMutableRawPointer?) -> Void)?
    let ctx: UnsafeMutableRawPointer?
}

private func copyCString(_ value: String) -> UnsafeMutablePointer<CChar>? {
    strdup(value)
}

private func jsonString(_ object: [String: Any]) -> String {
    guard JSONSerialization.isValidJSONObject(object),
          let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]),
          let text = String(data: data, encoding: .utf8)
    else {
        return #"{\"ok\":false,\"error\":\"JSONの作成に失敗しました\"}"#
    }
    return text
}

private func failure(_ message: String, cancelled: Bool = false) -> String {
    jsonString([
        "ok": false,
        "error": message,
        "cancelled": cancelled,
    ])
}

private func success(_ text: String) -> String {
    jsonString([
        "ok": true,
        "text": text,
    ])
}

private func describe(_ error: Error) -> (String, Bool) {
    if error is CancellationError {
        return (cancelledMessage, true)
    }
    if let generation = error as? LanguageModelSession.GenerationError {
        switch generation {
        case .exceededContextWindowSize:
            return ("入力が Apple Intelligence のコンテキスト上限を超えています。", false)
        case .assetsUnavailable:
            return ("Apple Intelligence のモデルを利用できません。", false)
        case .guardrailViolation:
            return ("Apple Intelligence の安全機能により応答できませんでした。", false)
        case .unsupportedLanguageOrLocale:
            return ("この言語は Apple Intelligence ではサポートされていません。", false)
        case .unsupportedGuide:
            return ("Apple Intelligence がこの出力形式に対応していません。", false)
        case .decodingFailure:
            return ("Apple Intelligence の応答を読み取れませんでした。", false)
        case .rateLimited:
            return ("Apple Intelligence が一時的に制限されています。しばらく待ってから再試行してください。", false)
        case .concurrentRequests:
            return ("Apple Intelligence は同時に1件までしか処理できません。", false)
        case .refusal:
            return ("Apple Intelligence がこの内容への応答を拒否しました。", false)
        @unknown default:
            return (generation.localizedDescription, false)
        }
    }
    if #available(macOS 27.0, *) {
        if let modelError = error as? LanguageModelError {
            switch modelError {
            case .contextSizeExceeded:
                return ("入力が Apple Intelligence のコンテキスト上限を超えています。", false)
            case .rateLimited:
                return ("Apple Intelligence が一時的に制限されています。しばらく待ってから再試行してください。", false)
            case .guardrailViolation:
                return ("Apple Intelligence の安全機能により応答できませんでした。", false)
            case .refusal:
                return ("Apple Intelligence がこの内容への応答を拒否しました。", false)
            case .unsupportedLanguageOrLocale:
                return ("この言語は Apple Intelligence ではサポートされていません。", false)
            case .timeout:
                return ("Apple Intelligence の応答がタイムアウトしました。", false)
            case .unsupportedCapability, .unsupportedTranscriptContent, .unsupportedGenerationGuide:
                return ("Apple Intelligence がこの要求に対応していません。", false)
            @unknown default:
                return (modelError.localizedDescription, false)
            }
        }
        if let session = error as? LanguageModelSession.Error {
            switch session {
            case .concurrentRequests:
                return ("Apple Intelligence は同時に1件までしか処理できません。", false)
            case .transcriptMutationWhileResponding:
                return ("Apple Intelligence の応答中に会話を変更できません。", false)
            @unknown default:
                return (session.localizedDescription, false)
            }
        }
    }
    let text = error.localizedDescription
    if text.isEmpty {
        return ("Apple Intelligence の推論に失敗しました。", false)
    }
    return (text, false)
}

private func modelDisplayName() -> String {
    if #available(macOS 27.0, *) {
        let name = SystemLanguageModel.default.variant.displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        if !name.isEmpty {
            return name
        }
    }
    return "Apple Intelligence"
}

private func availabilityObject() -> [String: Any] {
    let model = SystemLanguageModel.default
    switch model.availability {
    case .available:
        return [
            "supported": true,
            "reason": "",
            "permanent": false,
            "os": "macos",
            "model": modelDisplayName(),
            "context_size": model.contextSize,
        ]
    case .unavailable(let reason):
        let message: String
        let permanent: Bool
        switch reason {
        case .deviceNotEligible:
            message = "この Mac は Apple Intelligence に対応していません。"
            permanent = true
        case .appleIntelligenceNotEnabled:
            message = "Apple Intelligence がオフです。システム設定で有効にしてください。"
            permanent = false
        case .modelNotReady:
            message = "Apple Intelligence のモデルを準備中です。しばらく待ってからもう一度お試しください。"
            permanent = false
        @unknown default:
            message = "Apple Intelligence を利用できません。"
            permanent = false
        }
        return [
            "supported": false,
            "reason": message,
            "permanent": permanent,
            "os": "macos",
            "model": "",
            "context_size": 0,
        ]
    }
}

@_cdecl("selah_apple_ai_free")
public func selah_apple_ai_free(_ ptr: UnsafeMutablePointer<CChar>?) {
    free(ptr)
}

@_cdecl("selah_apple_ai_availability_json")
public func selah_apple_ai_availability_json() -> UnsafeMutablePointer<CChar>? {
    copyCString(jsonString(availabilityObject()))
}

@_cdecl("selah_apple_ai_cancel")
public func selah_apple_ai_cancel(_ genId: UnsafePointer<CChar>?) {
    guard let genId else { return }
    registry.cancel(String(cString: genId))
}

@_cdecl("selah_apple_ai_clear_cancel")
public func selah_apple_ai_clear_cancel(_ genId: UnsafePointer<CChar>?) {
    guard let genId else { return }
    registry.clear(String(cString: genId))
}

private func stringField(_ object: [String: Any], _ key: String) -> String {
    (object[key] as? String) ?? ""
}

private func doubleField(_ object: [String: Any], _ key: String) -> Double {
    if let value = object[key] as? Double { return value }
    if let value = object[key] as? Int { return Double(value) }
    if let value = object[key] as? NSNumber { return value.doubleValue }
    return 0.7
}

private func intField(_ object: [String: Any], _ key: String) -> Int {
    if let value = object[key] as? Int { return value }
    if let value = object[key] as? Double { return Int(value) }
    if let value = object[key] as? NSNumber { return value.intValue }
    return 0
}

private func boolField(_ object: [String: Any], _ key: String) -> Bool {
    if let value = object[key] as? Bool { return value }
    if let value = object[key] as? NSNumber { return value.boolValue }
    return false
}

private func emit(_ callback: SendableCallback, _ text: String, final isFinal: Bool) {
    guard let function = callback.callback else { return }
    text.withCString { pointer in
        function(pointer, isFinal ? 1 : 0, callback.ctx)
    }
}

private func delta(from previous: String, to next: String) -> String {
    if next.hasPrefix(previous) {
        return String(next.dropFirst(previous.count))
    }
    return next
}

private let contextOverheadTokens = 160
private let preferredResponseTokens = 768
private let jsonResponseTokens = 1280
private let minimumResponseTokens = 192
private let contextLimitNote = "\n\n（コンテキスト上限に達したため、ここまでの内容を残しました。）"

private func isContextLimit(_ error: Error) -> Bool {
    if error is CancellationError {
        return false
    }
    if let generation = error as? LanguageModelSession.GenerationError {
        if case .exceededContextWindowSize = generation {
            return true
        }
    }
    if #available(macOS 27.0, *) {
        if let modelError = error as? LanguageModelError, case .contextSizeExceeded = modelError {
            return true
        }
    }
    return false
}

private func estimateTokens(_ text: String) -> Int {
    if text.isEmpty { return 0 }
    var ascii = 0
    var other = 0
    for scalar in text.unicodeScalars {
        if scalar.value < 128 {
            ascii += 1
        } else {
            other += 1
        }
    }
    // Latin is about 3-4 characters per token. CJK is about one token per character.
    return other + (ascii + 2) / 3 + 8
}

private func countTokens(_ model: SystemLanguageModel, _ text: String) async -> Int {
    if text.isEmpty { return 0 }
    if #available(macOS 26.4, *) {
        if let count = try? await model.tokenCount(for: text), count >= 0 {
            return count
        }
    }
    return estimateTokens(text)
}

private struct TextAtom {
    var text: String
    var isBrace: Bool
    var index: Int
}

private func wantsJSONReply(_ instructions: String, _ prompt: String) -> Bool {
    let haystack = instructions + "\n" + prompt
    return haystack.contains("JSONのみ")
        || haystack.contains("JSONだけ")
        || haystack.contains("Output one JSON")
        || haystack.contains("出力は必ず")
}

private func jsonText(_ value: Any) -> String? {
    guard JSONSerialization.isValidJSONObject(value),
          let data = try? JSONSerialization.data(withJSONObject: value, options: []),
          let text = String(data: data, encoding: .utf8) else {
        return nil
    }
    return text
}

private func objectValue(_ value: Any) -> [String: Any]? {
    if let map = value as? [String: Any] {
        return map
    }
    guard let map = value as? NSDictionary else { return nil }
    var out: [String: Any] = [:]
    for key in map.allKeys {
        guard let name = key as? String, let item = map.object(forKey: key) else { return nil }
        out[name] = item
    }
    return out
}

private func arrayValue(_ value: Any) -> [Any]? {
    if let items = value as? [Any] {
        return items
    }
    if let items = value as? NSArray {
        return items.map { $0 }
    }
    return nil
}

private func renderedTokenCount(_ value: Any) -> Int {
    if let text = jsonText(value) {
        return estimateTokens(text)
    }
    if let text = value as? String {
        return estimateTokens(text) + 2
    }
    return estimateTokens(String(describing: value)) + 2
}

private func truncateJSONString(_ text: String, budget: Int) -> String {
    if budget < 2 { return "" }
    let chars = Array(text)
    var low = 0
    var high = chars.count
    while low < high {
        let mid = (low + high + 1) / 2
        let candidate = String(chars.prefix(mid))
        if estimateTokens(candidate) + 2 <= budget {
            low = mid
        } else {
            high = mid - 1
        }
    }
    var out = String(chars.prefix(low))
    if low < chars.count && estimateTokens(out + "…") + 2 <= budget {
        out += "…"
    }
    return out
}

private func compactJSONArray(_ items: [Any], budget: Int) -> [Any] {
    var kept: [Any] = []
    for item in items {
        let used = estimateTokens(jsonText(kept) ?? "[]")
        let remaining = budget - used - 1
        if remaining < 2 { break }
        let shrunk = compactJSONValue(item, budget: remaining)
        var trial = kept
        trial.append(shrunk)
        let rendered = jsonText(trial) ?? "[]"
        if estimateTokens(rendered) > budget { break }
        kept.append(shrunk)
    }
    return kept
}

private func compactJSONObject(_ map: [String: Any], budget: Int) -> [String: Any] {
    let entries = map.sorted { renderedTokenCount($0.value) < renderedTokenCount($1.value) }
    var kept: [String: Any] = [:]
    for (key, value) in entries {
        let used = estimateTokens(jsonText(kept) ?? "{}")
        let remaining = budget - used - estimateTokens(key) - 3
        if remaining < 2 { continue }
        let shrunk = compactJSONValue(value, budget: remaining)
        var trial = kept
        trial[key] = shrunk
        let rendered = jsonText(trial) ?? "{}"
        if estimateTokens(rendered) > budget { continue }
        kept[key] = shrunk
    }
    return kept
}

private func compactJSONValue(_ value: Any, budget: Int) -> Any {
    if budget < 2 { return NSNull() }
    if let text = jsonText(value), estimateTokens(text) <= budget {
        return value
    }
    if let text = value as? String {
        return truncateJSONString(text, budget: budget)
    }
    if let items = arrayValue(value) {
        return compactJSONArray(items, budget: budget)
    }
    if let map = objectValue(value) {
        return compactJSONObject(map, budget: budget)
    }
    if renderedTokenCount(value) <= budget {
        return value
    }
    return NSNull()
}

private func matchingClose(_ text: String, open: String.Index) -> String.Index? {
    guard open < text.endIndex else { return nil }
    let opener = text[open]
    let closer: Character = opener == "{" ? "}" : "]"
    var depth = 0
    var inString = false
    var escape = false
    var index = open
    while index < text.endIndex {
        let ch = text[index]
        if inString {
            if escape {
                escape = false
            } else if ch == "\\" {
                escape = true
            } else if ch == "\"" {
                inString = false
            }
        } else {
            switch ch {
            case "\"":
                inString = true
            case "{", "[":
                depth += 1
            case "}", "]":
                depth -= 1
                if depth == 0 {
                    let next = text.index(after: index)
                    return ch == closer ? next : nil
                }
            default:
                break
            }
        }
        index = text.index(after: index)
    }
    return nil
}

private func outermostBraceSpans(_ text: String) -> [Range<String.Index>] {
    var spans: [Range<String.Index>] = []
    var inString = false
    var escape = false
    var index = text.startIndex
    while index < text.endIndex {
        let ch = text[index]
        if inString {
            if escape {
                escape = false
            } else if ch == "\\" {
                escape = true
            } else if ch == "\"" {
                inString = false
            }
            index = text.index(after: index)
            continue
        }
        if ch == "\"" {
            inString = true
            index = text.index(after: index)
            continue
        }
        if ch == "{" || ch == "[" {
            if let end = matchingClose(text, open: index) {
                spans.append(index..<end)
                index = end
                continue
            }
        }
        index = text.index(after: index)
    }
    return spans
}

private func compactEmbeddedJSON(_ text: String, spanBudget: Int) -> String {
    let spans = outermostBraceSpans(text)
    if spans.isEmpty { return text }
    var out = ""
    var cursor = text.startIndex
    for span in spans {
        out += text[cursor..<span.lowerBound]
        let block = String(text[span])
        if let data = block.data(using: .utf8),
           let value = try? JSONSerialization.jsonObject(with: data),
           estimateTokens(block) > spanBudget {
            out += jsonText(compactJSONValue(value, budget: max(spanBudget, 16))) ?? "{}"
        } else {
            out += block
        }
        cursor = span.upperBound
    }
    out += text[cursor...]
    return out
}

private func splitAtoms(_ text: String) -> [TextAtom] {
    let spans = outermostBraceSpans(text)
    var atoms: [TextAtom] = []
    var cursor = text.startIndex
    func pushProse(_ prose: String) {
        if prose.isEmpty { return }
        let parts = prose.components(separatedBy: "\n\n")
        for (offset, part) in parts.enumerated() {
            if part.isEmpty { continue }
            let piece = offset == 0 ? part : "\n\n" + part
            if piece.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { continue }
            atoms.append(TextAtom(text: piece, isBrace: false, index: atoms.count))
        }
    }
    for span in spans {
        if cursor < span.lowerBound {
            pushProse(String(text[cursor..<span.lowerBound]))
        }
        atoms.append(TextAtom(text: String(text[span]), isBrace: true, index: atoms.count))
        cursor = span.upperBound
    }
    if cursor < text.endIndex {
        pushProse(String(text[cursor...]))
    }
    return atoms
}

private func sliceProse(_ text: String, budget: Int, keepTail: Bool) -> String {
    if estimateTokens(text) <= budget { return text }
    let chars = Array(text)
    var low = 0
    var high = chars.count
    while low < high {
        let mid = (low + high + 1) / 2
        let slice = keepTail ? String(chars.suffix(mid)) : String(chars.prefix(mid))
        if estimateTokens(slice) <= budget {
            low = mid
        } else {
            high = mid - 1
        }
    }
    if low == 0 { return "" }
    return keepTail ? String(chars.suffix(low)) : String(chars.prefix(low))
}

private func fitAtom(_ atom: TextAtom, budget: Int, keepTail: Bool) -> String? {
    if budget < 2 || atom.text.isEmpty { return nil }
    if estimateTokens(atom.text) <= budget { return atom.text }
    if atom.isBrace {
        let trimmed = atom.text.trimmingCharacters(in: .whitespacesAndNewlines)
        if let data = trimmed.data(using: .utf8),
           let value = try? JSONSerialization.jsonObject(with: data) {
            let compact = compactJSONValue(value, budget: budget)
            if let rendered = jsonText(compact), estimateTokens(rendered) <= budget {
                return rendered
            }
        }
        return nil
    }
    if atom.text.contains("{") || atom.text.contains("[") {
        return nil
    }
    let sliced = sliceProse(atom.text, budget: budget, keepTail: keepTail)
    if sliced.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return nil }
    return sliced
}

private func joinAtoms(_ atoms: [TextAtom]) -> String {
    var out = ""
    var previous: Int?
    for atom in atoms {
        if let previous, atom.index > previous + 1, !out.hasSuffix("…") {
            out += "\n…\n"
        }
        previous = atom.index
        out += atom.text
    }
    return out
}

private func atomRoom(_ budget: Int, used: Int) -> Int {
    let separator = used > 0 ? 1 : 0
    return max(budget - used - separator, 0)
}

private func selectSideAtoms(_ atoms: [TextAtom], budget: Int, keepTail: Bool) -> [TextAtom] {
    var chosen: [TextAtom] = []
    var used = 0
    let indexes = keepTail ? Array(atoms.indices.reversed()) : Array(atoms.indices)
    for index in indexes {
        let separator = used > 0 ? 1 : 0
        let remaining = atomRoom(budget, used: used)
        guard let fitted = fitAtom(atoms[index], budget: remaining, keepTail: keepTail) else { continue }
        let cost = estimateTokens(fitted) + separator
        if used + cost > budget { continue }
        used += cost
        var atom = atoms[index]
        atom.text = fitted
        if keepTail {
            chosen.insert(atom, at: 0)
        } else {
            chosen.append(atom)
        }
    }
    return chosen
}

private func selectHeadAndTailAtoms(_ atoms: [TextAtom], budget: Int) -> [TextAtom] {
    var chosen = Array(repeating: TextAtom?.none, count: atoms.count)
    var used = 0
    for (index, atom) in atoms.enumerated() where atom.isBrace {
        let separator = used > 0 ? 1 : 0
        let remaining = atomRoom(budget, used: used)
        guard let fitted = fitAtom(atom, budget: remaining, keepTail: false) else { continue }
        let cost = estimateTokens(fitted) + separator
        if used + cost > budget { continue }
        used += cost
        var kept = atom
        kept.text = fitted
        chosen[index] = kept
    }
    let proseBudget = max(budget - used, 0)
    let headLimit = proseBudget / 3
    var proseUsed = 0
    for (index, atom) in atoms.enumerated() {
        if chosen[index] != nil || atom.isBrace { continue }
        let separator = proseUsed > 0 ? 1 : 0
        let remaining = atomRoom(headLimit, used: proseUsed)
        guard let fitted = fitAtom(atom, budget: remaining, keepTail: false) else { break }
        let cost = estimateTokens(fitted) + separator
        if proseUsed + cost > headLimit { break }
        proseUsed += cost
        var kept = atom
        kept.text = fitted
        chosen[index] = kept
    }
    for (index, atom) in atoms.enumerated().reversed() {
        if chosen[index] != nil || atom.isBrace { continue }
        let separator = proseUsed > 0 ? 1 : 0
        let remaining = atomRoom(proseBudget, used: proseUsed)
        guard let fitted = fitAtom(atom, budget: remaining, keepTail: true) else { continue }
        let cost = estimateTokens(fitted) + separator
        if proseUsed + cost > proseBudget { continue }
        proseUsed += cost
        var kept = atom
        kept.text = fitted
        chosen[index] = kept
    }
    return chosen.compactMap { $0 }
}

private func indexToDrop(_ atoms: [TextAtom], dropFront: Bool) -> Int? {
    let prose = atoms.indices.filter { !atoms[$0].isBrace }
    if let index = dropFront ? prose.first : prose.last {
        return index
    }
    if atoms.isEmpty { return nil }
    return dropFront ? 0 : atoms.count - 1
}

private func trimStructured(
    _ model: SystemLanguageModel,
    _ text: String,
    budget: Int,
    keepTail: Bool
) async -> String {
    if budget <= 0 || text.isEmpty { return "" }
    if await countTokens(model, text) <= budget { return text }
    let compacted = compactEmbeddedJSON(text, spanBudget: max(budget, 32))
    if await countTokens(model, compacted) <= budget { return compacted }
    let atoms = splitAtoms(compacted)
    var chosen = keepTail
        ? selectSideAtoms(atoms, budget: budget, keepTail: true)
        : selectHeadAndTailAtoms(atoms, budget: budget)
    var safety = chosen.count + 2
    while safety > 0 && !chosen.isEmpty {
        let used = await countTokens(model, joinAtoms(chosen))
        if used <= budget { break }
        safety -= 1
        guard let index = indexToDrop(chosen, dropFront: keepTail) else { break }
        chosen.remove(at: index)
    }
    if chosen.count == 1, chosen[0].isBrace {
        let used = await countTokens(model, chosen[0].text)
        if used > budget {
            if let shrunk = fitAtom(chosen[0], budget: budget, keepTail: false) {
                let shrunkTokens = await countTokens(model, shrunk)
                if shrunkTokens <= budget {
                    return shrunk
                }
            }
            return ""
        }
    }
    return joinAtoms(chosen)
}

private func preShrink(_ text: String, budget: Int, keepTail: Bool) -> String {
    if budget <= 32 || text.isEmpty { return text }
    let estimated = estimateTokens(text)
    if estimated <= budget { return text }
    if text.contains("{") || text.contains("[") {
        return text
    }
    let ratio = Double(budget) / Double(max(estimated, 1))
    let keep = min(text.count, max(Int(Double(text.count) * ratio * 1.2), 96))
    let chars = Array(text)
    if keep >= chars.count { return text }
    if keepTail {
        return "…\n" + String(chars.suffix(keep))
    }
    return String(chars.prefix(keep)) + "\n…"
}

private func trimToBudget(
    _ model: SystemLanguageModel,
    _ text: String,
    budget: Int,
    keepTail: Bool
) async -> String {
    if budget <= 0 || text.isEmpty { return "" }
    if text.contains("{") || text.contains("[") {
        return await trimStructured(model, text, budget: budget, keepTail: keepTail)
    }
    let seeded = preShrink(text, budget: budget, keepTail: keepTail)
    if await countTokens(model, seeded) <= budget { return seeded }
    let chars = Array(seeded)
    var low = 0
    var high = chars.count
    var best = ""
    while low <= high {
        let mid = (low + high) / 2
        let slice = keepTail ? String(chars.suffix(mid)) : String(chars.prefix(mid))
        if await countTokens(model, slice) <= budget {
            best = slice
            low = mid + 1
        } else if mid == 0 {
            break
        } else {
            high = mid - 1
        }
    }
    if best.isEmpty { return "" }
    if best.count >= seeded.count { return best }
    return keepTail ? "…\n" + best : best + "\n…"
}

private func trimHeadAndTail(_ model: SystemLanguageModel, _ text: String, budget: Int) async -> String {
    if budget <= 0 || text.isEmpty { return "" }
    if await countTokens(model, text) <= budget { return text }
    if text.contains("{") || text.contains("[") {
        return await trimStructured(model, text, budget: budget, keepTail: false)
    }
    let headBudget = max(budget * 2 / 5, 48)
    let tailBudget = max(budget - headBudget - 8, 48)
    let head = await trimToBudget(model, text, budget: headBudget, keepTail: false)
    let tail = await trimToBudget(model, text, budget: tailBudget, keepTail: true)
    if head.isEmpty { return tail }
    if tail.isEmpty { return head }
    if head.count + tail.count >= text.count {
        return await trimToBudget(model, text, budget: budget, keepTail: true)
    }
    return head + "\n…\n" + tail
}

private func trimTurns(_ model: SystemLanguageModel, _ prompt: String, budget: Int) async -> String {
    if budget <= 0 || prompt.isEmpty { return "" }
    if await countTokens(model, prompt) <= budget { return prompt }
    if prompt.contains("{") || prompt.contains("[") {
        return await trimStructured(model, prompt, budget: budget, keepTail: true)
    }
    let parts = prompt.components(separatedBy: "\n\n").filter { !$0.isEmpty }
    if parts.count <= 1 {
        return await trimHeadAndTail(model, prompt, budget: budget)
    }
    var kept: [String] = [parts[parts.count - 1]]
    var used = await countTokens(model, kept[0])
    if used > budget {
        kept[0] = await trimHeadAndTail(model, kept[0], budget: budget)
        return kept[0]
    }
    for part in parts.dropLast().reversed() {
        let cost = await countTokens(model, part) + 2
        if used + cost > budget { break }
        used += cost
        kept.insert(part, at: 0)
    }
    if kept.count == parts.count {
        return kept.joined(separator: "\n\n")
    }
    return "…\n\n" + kept.joined(separator: "\n\n")
}

private struct FittedRequest {
    var instructions: String
    var prompt: String
    var responseTokens: Int
    var trimmed: Bool
}

private func fitToContext(
    model: SystemLanguageModel,
    instructions: String,
    prompt: String,
    requestedMaxTokens: Int
) async -> FittedRequest {
    let contextSize = model.contextSize > 256 ? model.contextSize : 4096
    var fittedInstructions = instructions
    var fittedPrompt = prompt.isEmpty ? "応答してください。" : prompt
    var trimmed = false
    let responseReserve = wantsJSONReply(instructions, prompt) ? jsonResponseTokens : preferredResponseTokens

    func inputTokens() async -> Int {
        let instructionsCount = await countTokens(model, fittedInstructions)
        let promptCount = await countTokens(model, fittedPrompt)
        return instructionsCount + promptCount + 24
    }

    var used = await inputTokens()
    var remaining = contextSize - used - contextOverheadTokens
    if remaining < minimumResponseTokens {
        trimmed = true
        let inputBudget = max(contextSize - responseReserve - contextOverheadTokens, 512)
        let promptCount = await countTokens(model, fittedPrompt)
        let promptBudget = max(min(inputBudget / 2, promptCount), 160)
        fittedPrompt = await trimTurns(model, fittedPrompt, budget: promptBudget)
        let fittedPromptCount = await countTokens(model, fittedPrompt)
        let instructionBudget = max(inputBudget - fittedPromptCount - 24, 128)
        if await countTokens(model, fittedInstructions) > instructionBudget {
            fittedInstructions = await trimHeadAndTail(model, fittedInstructions, budget: instructionBudget)
        }
        used = await inputTokens()
        remaining = contextSize - used - contextOverheadTokens
        if remaining < minimumResponseTokens {
            fittedPrompt = await trimTurns(model, fittedPrompt, budget: max(inputBudget / 3, 96))
            let tighterPromptCount = await countTokens(model, fittedPrompt)
            let tighterInstructions = max(inputBudget - tighterPromptCount - 24, 64)
            fittedInstructions = await trimHeadAndTail(model, fittedInstructions, budget: tighterInstructions)
            used = await inputTokens()
            remaining = contextSize - used - contextOverheadTokens
        }
    }

    var responseTokens = max(remaining, 32)
    if requestedMaxTokens > 0 {
        responseTokens = min(responseTokens, requestedMaxTokens)
    }
    responseTokens = max(responseTokens, 32)
    if fittedPrompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        fittedPrompt = "応答してください。"
    }
    return FittedRequest(
        instructions: fittedInstructions.trimmingCharacters(in: .whitespacesAndNewlines),
        prompt: fittedPrompt,
        responseTokens: responseTokens,
        trimmed: trimmed
    )
}

private struct StreamOutcome {
    var text: String
    var hitContextLimit: Bool
}

private func streamText(
    instructions: String,
    prompt: String,
    options: GenerationOptions,
    genId: String,
    callback: SendableCallback
) async throws -> StreamOutcome {
    let session: LanguageModelSession
    if instructions.isEmpty {
        session = LanguageModelSession()
    } else {
        session = LanguageModelSession(instructions: instructions)
    }
    let stream = session.streamResponse(to: prompt, options: options)
    var previous = ""
    do {
        for try await snapshot in stream {
            try Task.checkCancellation()
            if registry.isCancelled(genId) {
                throw CancellationError()
            }
            let content = snapshot.content
            let piece = delta(from: previous, to: content)
            previous = content
            if !piece.isEmpty {
                emit(callback, piece, final: false)
            }
        }
        return StreamOutcome(text: previous, hitContextLimit: false)
    } catch {
        if isContextLimit(error), !previous.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return StreamOutcome(text: previous, hitContextLimit: true)
        }
        throw error
    }
}

private func finishPartial(_ text: String, _ callback: SendableCallback) -> String {
    emit(callback, contextLimitNote, final: false)
    emit(callback, "", final: true)
    return success(text + contextLimitNote)
}

private func generationOptions(temperature: Double, greedy: Bool, responseTokens: Int) -> GenerationOptions {
    var options = GenerationOptions()
    if greedy {
        options.samplingMode = .greedy
    } else {
        options.temperature = min(max(temperature, 0), 2)
    }
    // An unset cap lets generation run until the window is full, then the
    // framework throws and the whole send fails. Always stop inside the window.
    options.maximumResponseTokens = max(responseTokens, 32)
    return options
}

private func runGenerate(_ raw: String, _ callback: SendableCallback) async -> String {
    guard let data = raw.data(using: .utf8),
          let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
    else {
        return failure("推論リクエストを読み取れませんでした。")
    }

    let genId = stringField(object, "gen_id")
    if registry.isCancelled(genId) || Task.isCancelled {
        return failure(cancelledMessage, cancelled: true)
    }

    let model = SystemLanguageModel.default
    guard model.isAvailable else {
        let reason = availabilityObject()["reason"] as? String ?? "Apple Intelligence を利用できません。"
        return failure(reason)
    }

    let instructions = stringField(object, "instructions").trimmingCharacters(in: .whitespacesAndNewlines)
    var prompt = stringField(object, "prompt").trimmingCharacters(in: .whitespacesAndNewlines)
    if prompt.isEmpty {
        prompt = instructions.isEmpty ? "応答してください。" : "指示に従って応答してください。"
    }

    let temperature = doubleField(object, "temperature")
    let maxTokens = intField(object, "max_tokens")
    let greedy = boolField(object, "greedy") || temperature <= 0
    let fitted = await fitToContext(
        model: model,
        instructions: instructions,
        prompt: prompt,
        requestedMaxTokens: maxTokens
    )
    if fitted.trimmed {
        NSLog("Selah Apple Intelligence trimmed the prompt to leave room for a response")
    }

    let options = generationOptions(
        temperature: temperature,
        greedy: greedy,
        responseTokens: fitted.responseTokens
    )
    do {
        let outcome = try await streamText(
            instructions: fitted.instructions,
            prompt: fitted.prompt,
            options: options,
            genId: genId,
            callback: callback
        )
        if outcome.hitContextLimit {
            return finishPartial(outcome.text, callback)
        }
        emit(callback, "", final: true)
        return success(outcome.text)
    } catch {
        if registry.isCancelled(genId) || error is CancellationError {
            return failure(cancelledMessage, cancelled: true)
        }
        guard isContextLimit(error) else {
            let (message, cancelled) = describe(error)
            return failure(message, cancelled: cancelled)
        }

        let instructionBudget = max(await countTokens(model, fitted.instructions) / 2, 64)
        let promptBudget = max(await countTokens(model, fitted.prompt) / 2, 64)
        let shrunkInstructions = await trimHeadAndTail(model, fitted.instructions, budget: instructionBudget)
        let shrunkPrompt = await trimTurns(model, fitted.prompt, budget: promptBudget)
        let retryOptions = generationOptions(
            temperature: temperature,
            greedy: greedy,
            responseTokens: min(fitted.responseTokens, minimumResponseTokens)
        )
        do {
            let outcome = try await streamText(
                instructions: shrunkInstructions,
                prompt: shrunkPrompt,
                options: retryOptions,
                genId: genId,
                callback: callback
            )
            if outcome.hitContextLimit {
                return finishPartial(outcome.text, callback)
            }
            emit(callback, "", final: true)
            return success(outcome.text)
        } catch let retryError {
            if registry.isCancelled(genId) || retryError is CancellationError {
                return failure(cancelledMessage, cancelled: true)
            }
            let (message, cancelled) = describe(retryError)
            return failure(message, cancelled: cancelled)
        }
    }
}

@_cdecl("selah_apple_ai_generate")
public func selah_apple_ai_generate(
    _ requestJson: UnsafePointer<CChar>?,
    _ callback: (@convention(c) (UnsafePointer<CChar>?, Int32, UnsafeMutableRawPointer?) -> Void)?,
    _ ctx: UnsafeMutableRawPointer?
) -> UnsafeMutablePointer<CChar>? {
    guard let requestJson else {
        return copyCString(failure("推論リクエストが空です。"))
    }
    let raw = String(cString: requestJson)
    let genId: String = {
        guard let data = raw.data(using: .utf8),
              let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        else { return "" }
        return stringField(object, "gen_id")
    }()
    let box = SendableCallback(callback: callback, ctx: ctx)
    let result = UnsafeMutablePointer<String>.allocate(capacity: 1)
    result.initialize(to: "")
    let semaphore = DispatchSemaphore(value: 0)
    let task = Task {
        let value = await runGenerate(raw, box)
        result.pointee = value
        semaphore.signal()
    }
    registry.insert(genId, task)
    semaphore.wait()
    registry.remove(genId)
    let json = result.pointee
    result.deinitialize(count: 1)
    result.deallocate()
    return copyCString(json)
}
