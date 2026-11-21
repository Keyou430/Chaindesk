package dev.chaindesk.core

import org.json.JSONObject

class CoreFailure(val code: String) : Exception(code)

object CoreEngine {
    fun importSubscription(source: ByteArray): JSONObject = result { NativeBridge.importSubscription(source) }
    fun compile(input: JSONObject): JSONObject = result { NativeBridge.compileChain(input.toString().toByteArray(Charsets.UTF_8)) }
    fun ruleManifest(): JSONObject = result { NativeBridge.ruleManifest() }
    fun setRulesDirectory(path: String): JSONObject = result {
        NativeBridge.setRulesDirectory(JSONObject().put("path", path).toString().toByteArray(Charsets.UTF_8))
    }
    fun probe(id: Long, path: String, expectedIpv4: String, budgetMillis: Long): JSONObject = result {
        val input = JSONObject().put("path", path).put("expectedIpv4", expectedIpv4).put("budgetMillis", budgetMillis.coerceIn(1, 30_000))
        NativeBridge.probe(id, input.toString().toByteArray(Charsets.UTF_8))
    }

    private fun result(operation: () -> ByteArray): JSONObject {
        val decoded = try {
            JSONObject(operation().toString(Charsets.UTF_8))
        } catch (_: LinkageError) {
            throw CoreFailure("CORE_LOAD_FAILED")
        } catch (_: Exception) {
            throw CoreFailure("CORE_RESPONSE_INVALID")
        }
        if (decoded.has("errorCode")) throw CoreFailure(decoded.getString("errorCode"))
        return decoded
    }
}
