package dev.chaindesk.android.profile

import android.content.Context
import android.util.AtomicFile
import dev.chaindesk.core.CoreEngine
import dev.chaindesk.core.CoreFailure
import org.json.JSONObject
import java.io.File
import java.security.MessageDigest

object RuleAssets {
    @Synchronized fun prepare(context: Context): String {
        val expected = CoreEngine.ruleManifest()
        val packaged = context.assets.open("rules/manifest.json").use { JSONObject(it.readBytes().toString(Charsets.UTF_8)) }
        val version = expected.getString("ruleVersion")
        if (packaged.getString("ruleVersion") != version || !version.matches(Regex("[a-zA-Z0-9-]+"))) throw CoreFailure("RULE_RESOURCE_UNAVAILABLE")
        val directory = File(context.filesDir, "rules-$version")
        if (!directory.exists() && !directory.mkdir()) throw CoreFailure("RULE_RESOURCE_UNAVAILABLE")
        val files = expected.getJSONArray("files")
        for (index in 0 until files.length()) {
            val entry = files.getJSONObject(index)
            val name = entry.getString("name")
            if (name !in setOf("geoip.dat", "geosite.dat")) throw CoreFailure("RULE_RESOURCE_UNAVAILABLE")
            val size = entry.getLong("bytes")
            val hash = entry.getString("sha256")
            val destination = File(directory, name)
            if (!destination.exists()) {
                val atomic = AtomicFile(destination)
                val output = atomic.startWrite()
                try {
                    val digest = MessageDigest.getInstance("SHA-256")
                    var count = 0L
                    context.assets.open("rules/$name").use { input ->
                        val buffer = ByteArray(8192)
                        while (true) {
                            val read = input.read(buffer)
                            if (read == -1) break
                            count += read
                            if (count > size) throw CoreFailure("RULE_RESOURCE_UNAVAILABLE")
                            digest.update(buffer, 0, read)
                            output.write(buffer, 0, read)
                        }
                    }
                    if (count != size || digest.digest().hex() != hash) throw CoreFailure("RULE_RESOURCE_UNAVAILABLE")
                    output.fd.sync()
                    atomic.finishWrite(output)
                } catch (_: Exception) {
                    atomic.failWrite(output)
                    throw CoreFailure("RULE_RESOURCE_UNAVAILABLE")
                }
            }
            val digest = MessageDigest.getInstance("SHA-256")
            if (destination.length() != size) throw CoreFailure("RULE_RESOURCE_UNAVAILABLE")
            destination.inputStream().use { input ->
                val buffer = ByteArray(8192)
                while (true) {
                    val count = input.read(buffer)
                    if (count == -1) break
                    digest.update(buffer, 0, count)
                }
            }
            if (digest.digest().hex() != hash) throw CoreFailure("RULE_RESOURCE_UNAVAILABLE")
        }
        CoreEngine.setRulesDirectory(directory.absolutePath)
        return version
    }

    private fun ByteArray.hex(): String = joinToString("") { "%02x".format(it) }
}
