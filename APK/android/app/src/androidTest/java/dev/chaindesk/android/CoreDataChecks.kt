package dev.chaindesk.android

import android.content.Context
import dev.chaindesk.android.profile.RuleAssets
import dev.chaindesk.core.NativeBridge
import org.json.JSONObject
import java.io.File
import java.io.RandomAccessFile

object CoreDataChecks {
    fun run(context: Context) {
        val source = "proxies: [{name: '中文 🔗 同名', type: ss, server: 192.0.2.8, port: 443, cipher: aes-128-gcm, password: '密码&<>'}]"
        val imported = JSONObject(NativeBridge.importSubscription(source.toByteArray()).toString(Charsets.UTF_8))
        check(!imported.has("errorCode")) { "Native subscription import failed" }
        val front = imported.getJSONArray("nodes").getJSONObject(0)
        check(front.getString("name") == "中文 🔗 同名") { "JNI changed supplementary UTF-8" }
        val input = JSONObject().put("front", front).put("preset", "allProxy").put("exit",
            JSONObject().put("host", "192.0.2.9").put("port", 1080).put("username", "用户").put("password", "密码"))
        val compiled = JSONObject(NativeBridge.compileChain(input.toString().toByteArray()).toString(Charsets.UTF_8))
        check(!compiled.has("errorCode")) { "Native chain compile failed" }
        check(JSONObject(compiled.getString("config")).getJSONObject("tun").getBoolean("enable").not())
        val invalid = JSONObject(NativeBridge.importSubscription("proxies: [*secret]".toByteArray()).toString(Charsets.UTF_8))
        check(invalid.has("errorCode") && !invalid.toString().contains("secret")) { "Parse error exposed source data" }
        val version = RuleAssets.prepare(context)
        input.put("preset", "chinaDirect").put("ruleVersion", version)
        val domestic = JSONObject(NativeBridge.compileChain(input.toString().toByteArray()).toString(Charsets.UTF_8))
        check(!domestic.has("errorCode")) { "Bundled native rules did not load" }
        val file = File(context.filesDir, "rules-$version/geosite.dat")
        RandomAccessFile(file, "rw").use { data ->
            val original = data.readByte()
            try {
                data.seek(0)
                data.writeByte(original.toInt() xor 1)
                val rejected = JSONObject(NativeBridge.compileChain(input.toString().toByteArray()).toString(Charsets.UTF_8))
                check(rejected.optString("errorCode") == "RULE_RESOURCE_UNAVAILABLE") { "Corrupted rules reached native parser" }
                data.seek(0)
                check(data.readByte() != original) { "Native parser replaced a corrupted resource" }
            } finally {
                data.seek(0)
                data.writeByte(original.toInt())
            }
        }
        RuleAssets.prepare(context)
    }
}
