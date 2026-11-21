package dev.chaindesk.android.session

import org.junit.Assert.*
import org.junit.Test

class SessionControllerTest {
    @Test fun requiredIdentityFailureBlocksButOptionalObservationDoesNot() {
        val controller = SessionController()
        val generation = controller.submit(SessionIntent.Connect("front-a")).generation
        controller.submit(SessionIntent.Started(generation, 9))
        controller.submit(SessionIntent.Verified(generation, null, 9))
        controller.submit(SessionIntent.HealthFailed(generation, 9, "IP_UNAVAILABLE", false))
        assertEquals(SessionPhase.Connected, controller.state.value.phase)
        controller.submit(SessionIntent.HealthFailed(generation, 8, "IP_UNAVAILABLE", true))
        assertEquals(SessionPhase.Connected, controller.state.value.phase)
        controller.submit(SessionIntent.HealthFailed(generation, 9, "IP_INCONSISTENT", true))
        assertEquals(SessionPhase.Degraded, controller.state.value.phase)
        assertEquals(0L, controller.state.value.nativeSessionId)
        assertTrue(controller.state.value.tunCaptured)
        assertEquals(generation, controller.state.value.generation)
    }

    @Test fun threeConsecutiveReachabilityFailuresDegradeAndSuccessResetsThem() {
        val controller = SessionController()
        val generation = controller.submit(SessionIntent.Connect("front-a")).generation
        controller.submit(SessionIntent.Started(generation, 9))
        controller.submit(SessionIntent.Verified(generation, null, 9))
        repeat(2) { controller.submit(SessionIntent.HealthFailed(generation, 9, "PROBE_TIMEOUT", false)) }
        assertEquals(SessionPhase.Connected, controller.state.value.phase)
        controller.submit(SessionIntent.Observation(generation, 9, null, null))
        repeat(2) { controller.submit(SessionIntent.HealthFailed(generation, 9, "PROBE_TIMEOUT", false)) }
        assertEquals(SessionPhase.Connected, controller.state.value.phase)
        controller.submit(SessionIntent.HealthFailed(generation, 9, "PROBE_TIMEOUT", false))
        assertEquals(SessionPhase.Degraded, controller.state.value.phase)
    }

    @Test fun staleNativeNetworkEventCannotInvalidateNewCandidate() {
        val controller = SessionController()
        val generation = controller.submit(SessionIntent.Connect("front-a")).generation
        controller.submit(SessionIntent.Started(generation, 9))
        controller.submit(SessionIntent.Trying(generation, "front-b"))
        controller.submit(SessionIntent.Started(generation, 10))
        controller.submit(SessionIntent.NetworkLost(generation, 9))
        assertEquals(SessionPhase.Probing, controller.state.value.phase)
        controller.submit(SessionIntent.NetworkLost(generation, 10))
        assertEquals(SessionPhase.Degraded, controller.state.value.phase)
    }

    @Test fun nativeActivationAndVerifiedStateHaveOneOwner() {
        val controller = SessionController()
        val generation = controller.submit(SessionIntent.Connect("front-a")).generation
        controller.submit(SessionIntent.Started(generation, 9))
        var activated = 0
        assertFalse(controller.verifyAndActivate(generation, 8, null) { activated++; true })
        assertEquals(0, activated)
        assertFalse(controller.verifyAndActivate(generation, 9, null) { false })
        assertEquals(SessionPhase.Probing, controller.state.value.phase)
        controller.submit(SessionIntent.NetworkLost(generation, 9))
        assertFalse(controller.verifyAndActivate(generation, 9, null) { activated++; true })
        assertEquals(0, activated)
        val replacement = controller.submit(SessionIntent.Switch("front-b")).generation
        controller.submit(SessionIntent.Started(replacement, 10))
        assertTrue(controller.verifyAndActivate(replacement, 10, "203.0.113.8") { activated++; true })
        assertEquals(1, activated)
        assertEquals(SessionPhase.Connected, controller.state.value.phase)
    }

    @Test fun staleNativeProbeCannotVerifyTheNextCandidate() {
        val controller = SessionController()
        val generation = controller.submit(SessionIntent.Connect("front-a")).generation
        controller.submit(SessionIntent.Started(generation, 9))
        controller.submit(SessionIntent.Trying(generation, "front-b"))
        controller.submit(SessionIntent.Started(generation, 10))
        controller.submit(SessionIntent.Verified(generation, "203.0.113.8", 9))
        assertEquals(SessionPhase.Probing, controller.state.value.phase)
        controller.submit(SessionIntent.Verified(generation, "203.0.113.8", 10))
        assertEquals(SessionPhase.Connected, controller.state.value.phase)
    }

    @Test fun disconnectInvalidatesPendingSuccess() {
        val controller = SessionController()
        val attempt = controller.submit(SessionIntent.Connect("front-a")).generation
        controller.submit(SessionIntent.Started(attempt, 8))
        val stopping = controller.submit(SessionIntent.Disconnect)
        controller.submit(SessionIntent.Verified(attempt, "203.0.113.8", 8))
        assertEquals(SessionPhase.Disconnecting, controller.state.value.phase)
        assertTrue(stopping.generation > attempt)
        controller.submit(SessionIntent.Stopped(stopping.generation))
        assertFalse(controller.state.value.tunCaptured)
        assertEquals(SessionPhase.Disconnected, controller.state.value.phase)
    }

    @Test fun duplicateConnectCannotStartASecondSession() {
        val controller = SessionController()
        val first = controller.submit(SessionIntent.Connect("front-a"))
        assertEquals(first, controller.submit(SessionIntent.Connect("front-b")))
        controller.submit(SessionIntent.Started(first.generation, 9))
        controller.submit(SessionIntent.Verified(first.generation, null, 9))
        assertEquals(SessionPhase.Connected, controller.state.value.phase)
        assertEquals(first.generation, controller.submit(SessionIntent.Connect("front-b")).generation)
    }

    @Test fun nativeStartAloneDoesNotPublishConnected() {
        val controller = SessionController()
        val first = controller.submit(SessionIntent.Connect("front-a"))
        controller.submit(SessionIntent.Verified(first.generation, "203.0.113.8", 9))
        assertEquals(SessionPhase.Connecting, controller.state.value.phase)
        controller.submit(SessionIntent.Started(first.generation, 9))
        assertEquals(SessionPhase.Probing, controller.state.value.phase)
        assertTrue(controller.state.value.tunCaptured)
    }

    @Test fun networkLossPreventsLateProbeSuccessAndKeepsCapture() {
        val controller = SessionController()
        val first = controller.submit(SessionIntent.Connect("front-a"))
        controller.submit(SessionIntent.Started(first.generation, 9))
        controller.submit(SessionIntent.NetworkLost(first.generation))
        controller.submit(SessionIntent.Verified(first.generation, "203.0.113.8", 9))
        assertEquals(SessionPhase.Degraded, controller.state.value.phase)
        assertTrue(controller.state.value.tunCaptured)
        assertEquals("NETWORK_LOST", controller.state.value.errorCode)
    }

    @Test fun switchInvalidatesPriorAttemptAndRetainsTheTun() {
        val controller = SessionController()
        val first = controller.submit(SessionIntent.Connect("front-a"))
        controller.submit(SessionIntent.Started(first.generation, 9))
        controller.submit(SessionIntent.Verified(first.generation, "203.0.113.8", 9))
        val replacement = controller.submit(SessionIntent.Switch("front-b"))
        assertTrue(replacement.generation > first.generation)
        assertTrue(replacement.tunCaptured)
        controller.submit(SessionIntent.Failed(first.generation, "OLD_FAILURE", true))
        assertNull(controller.state.value.errorCode)
        assertEquals("front-b", controller.state.value.frontId)
    }
}
