package dev.chaindesk.android

import android.app.Instrumentation
import android.os.Build
import android.os.SystemClock
import dev.chaindesk.android.profile.ProfileRepository
import dev.chaindesk.android.session.SessionIntent
import dev.chaindesk.android.session.SessionPhase
import java.io.File

internal object ModelCommitChecks {
    fun run(instrumentation: Instrumentation, scenario: String) {
        check(Build.HARDWARE == "ranchu") { "Dedicated emulator required" }
        instrumentation.waitForIdleSync()
        val application = instrumentation.targetContext.applicationContext as ChainDeskApplication
        val model = application.model
        fun await(condition: () -> Boolean) {
            val deadline = SystemClock.elapsedRealtime() + 15_000
            while (!condition()) {
                check(SystemClock.elapsedRealtime() < deadline) { "Model transaction timed out" }
                SystemClock.sleep(10)
            }
        }
        fun edit(operation: () -> Unit) {
            operation()
            await { !model.state.value.busy }
            check(model.state.value.errorCode == null) { "Model transaction failed" }
        }
        fun disconnect() {
            val state = model.sessions.submit(SessionIntent.Disconnect)
            model.sessions.submit(SessionIntent.Stopped(state.generation))
        }
        await { model.state.value.loaded && !model.state.value.busy }
        check(model.state.value.profile.subscription.isEmpty() && !model.sessions.state.value.active) { "Empty test profile required" }
        val history = ProfileRepository(File(application.noBackupFilesDir, "last-verified-v1.bin"), "${application.packageName}.last-verified.v1")
        try {
            edit { model.importFile("""
                proxies:
                  - {name: A, type: ss, server: 127.0.0.1, port: 18388, cipher: aes-128-gcm, password: synthetic-front-A}
                  - {name: B, type: ss, server: 127.0.0.1, port: 18389, cipher: aes-128-gcm, password: synthetic-front-B}
            """.trimIndent().toByteArray()) }
            val first = model.state.value.nodes[0]
            val second = model.state.value.nodes[1]
            val profile = model.state.value.profile.copy(exitHost = "127.0.0.1", exitPort = 18080,
                exitUsername = "synthetic-user", exitPassword = "synthetic-exit", rememberCredentials = true, exitAuthenticationRequired = true)
            edit { model.saveProfile(profile) }
            val token = model.sessions.submit(SessionIntent.Connect(first.id)).generation
            model.sessions.submit(SessionIntent.Started(token, 41))
            check(model.commitVerified(profile, first, token, 41, { true }) {
                model.sessions.verifyAndActivate(token, 41, null) { true }
            }) { "Initial verified record failed" }
            disconnect()
            when (scenario) {
                "excludedRollback" -> {
                    edit { model.excludeFront(first.id, true) }
                    check(model.lastVerifiedFor(model.state.value.profile) == null) { "Excluded historical front remained eligible for rollback" }
                }
                "invalidCommit" -> {
                    val next = model.sessions.submit(SessionIntent.Connect(second.id)).generation
                    model.sessions.submit(SessionIntent.Started(next, 42))
                    model.sessions.submit(SessionIntent.NetworkLost(next))
                    check(!model.commitVerified(model.state.value.profile, second, next, 42, { true }) {
                        model.sessions.verifyAndActivate(next, 42, null) { true }
                    }) { "Network-invalidated session committed selection" }
                    check(ProfileRepository(application).load()!!.selectedFrontId == first.id && history.load()!!.selectedFrontId == first.id) { "Invalid commit changed persistent records" }
                }
                "forgetHistory" -> {
                    edit { model.saveProfile(model.state.value.profile.copy(rememberCredentials = false)) }
                    val stored = history.load()!!
                    check(stored.exitUsername.isEmpty() && stored.exitPassword.isEmpty() && !stored.rememberCredentials) { "Historical vault retained forgotten credentials" }
                }
                "cancelAfterPrimary", "cancelAfterHistory", "deadlineAfterHistory", "networkAfterHistory", "activationFailure" -> {
                    val next = model.sessions.submit(SessionIntent.Connect(second.id)).generation
                    model.sessions.submit(SessionIntent.Started(next, 42))
                    var checks = 0
                    val valid = {
                        checks++
                        val failAt = if (scenario == "cancelAfterPrimary") 2 else 3
                        if (checks == failAt && scenario != "activationFailure") {
                            when (scenario) {
                                "cancelAfterPrimary", "cancelAfterHistory" -> model.sessions.submit(SessionIntent.Disconnect)
                                "networkAfterHistory" -> model.sessions.submit(SessionIntent.NetworkLost(next, 42))
                            }
                            false
                        } else true
                    }
                    check(!model.commitVerified(model.state.value.profile, second, next, 42, valid) {
                        if (scenario == "activationFailure") false else model.sessions.verifyAndActivate(next, 42, null) { true }
                    }) { "Interrupted transaction activated" }
                    check(ProfileRepository(application).load()!!.selectedFrontId == first.id && history.load()!!.selectedFrontId == first.id &&
                        model.state.value.profile.selectedFrontId == first.id) { "Interrupted transaction changed profile or historical record" }
                }
                "currentPolicyRollback" -> {
                    edit { model.saveProfile(model.state.value.profile.copy(automaticFront = true, healthMonitoring = true, excludedFrontIds = setOf(second.id))) }
                    val current = model.state.value.profile
                    val recovery = model.lastVerifiedFor(current)!!
                    check(recovery.first == current && recovery.second.id == first.id) { "Rollback overwrote current policy" }
                }
                else -> error("Unknown model scenario")
            }
        } finally {
            disconnect()
            edit { model.resetProfile() }
        }
    }
}
