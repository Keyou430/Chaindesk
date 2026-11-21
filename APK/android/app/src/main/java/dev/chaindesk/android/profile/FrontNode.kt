package dev.chaindesk.android.profile

import org.json.JSONObject

data class FrontNode(
    val id: String,
    val name: String,
    val protocol: String,
    val server: String,
    val port: Int,
    val rejection: String,
    val document: String,
) {
    companion object {
        fun from(value: JSONObject) = FrontNode(
            value.getString("id"), value.getString("name"), value.getString("protocol"),
            value.getString("server"), value.getInt("port"), value.optString("rejection"), value.toString(),
        )
    }
}
