package io.github.cyberbasslord666.termuxmcpedge.bridge;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotSame;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

public final class BridgeParcelableContractTest {
    @Test
    public void contextRejectsUnknownVersionAndNonPositiveRequestId() {
        assertThrows(
                IllegalArgumentException.class,
                () -> context(BridgeCallContext.PROTOCOL_VERSION + 1, 1L));
        assertThrows(
                IllegalArgumentException.class,
                () -> context(BridgeCallContext.PROTOCOL_VERSION, 0L));
    }

    @Test
    public void contextRejectsEveryNonExactDigestOrNonceLength() {
        byte[] fixed = bytes(32, 1);
        assertThrows(
                IllegalArgumentException.class,
                () -> new BridgeCallContext(1, 1L, new byte[31], fixed, fixed, fixed));
        assertThrows(
                IllegalArgumentException.class,
                () -> new BridgeCallContext(1, 1L, fixed, new byte[33], fixed, fixed));
        assertThrows(
                IllegalArgumentException.class,
                () -> new BridgeCallContext(1, 1L, fixed, fixed, new byte[0], fixed));
        assertThrows(
                IllegalArgumentException.class,
                () -> new BridgeCallContext(1, 1L, fixed, fixed, fixed, null));
    }

    @Test
    public void everyParcelableCreatorBoundsArrayAllocation() {
        assertEquals(16, BridgeCallContext.CREATOR.newArray(16).length);
        assertEquals(0, BridgeFailure.CREATOR.newArray(0).length);
        assertEquals(1, BridgeIdentityObservation.CREATOR.newArray(1).length);
        assertEquals(2, SystemFeaturesResult.CREATOR.newArray(2).length);
        assertThrows(
                IllegalArgumentException.class,
                () -> BridgeCallContext.CREATOR.newArray(-1));
        assertThrows(
                IllegalArgumentException.class,
                () -> BridgeFailure.CREATOR.newArray(17));
        assertThrows(
                IllegalArgumentException.class,
                () -> BridgeIdentityObservation.CREATOR.newArray(Integer.MAX_VALUE));
        assertThrows(
                IllegalArgumentException.class,
                () -> SystemFeaturesResult.CREATOR.newArray(Integer.MAX_VALUE));
    }

    @Test
    public void mutableInputAndOutputArraysNeverAliasContextStorage() {
        byte[] nonce = bytes(32, 2);
        byte[] manifest = bytes(32, 3);
        byte[] build = bytes(32, 4);
        byte[] contract = bytes(32, 5);
        BridgeCallContext context =
                new BridgeCallContext(1, 7L, nonce, manifest, build, contract);
        nonce[0] = 99;
        byte[] observed = context.getRequestNonce();
        assertEquals(2, observed[0]);
        observed[0] = 100;
        assertEquals(2, context.getRequestNonce()[0]);
        assertNotSame(observed, context.getRequestNonce());
    }

    @Test
    public void failureCodeSetIsClosed() {
        BridgeCallContext context = context(1, 9L);
        assertThrows(
                NullPointerException.class,
                () -> new BridgeFailure(
                        null,
                        bytes(32, 1),
                        bytes(32, 2),
                        BridgeFailure.INVALID_REQUEST,
                        false));
        assertThrows(
                IllegalArgumentException.class,
                () -> new BridgeFailure(context, bytes(32, 1), bytes(32, 2), 0, false));
        assertThrows(
                IllegalArgumentException.class,
                () -> new BridgeFailure(context, bytes(32, 1), bytes(32, 2), 9, false));
        BridgeFailure failure =
                new BridgeFailure(
                        context,
                        bytes(32, 1),
                        bytes(32, 2),
                        BridgeFailure.LIFECYCLE_CHANGED,
                        true);
        assertEquals(BridgeFailure.LIFECYCLE_CHANGED, failure.getFailureCode());
        assertTrue(failure.isRetryable());
    }

    @Test
    public void failureAndIdentityCloneEveryMutableInputAndOutput() {
        BridgeCallContext context = context(1, 9L);
        byte[] failureBuild = bytes(32, 21);
        byte[] failureContract = bytes(32, 22);
        BridgeFailure failure =
                new BridgeFailure(
                        context,
                        failureBuild,
                        failureContract,
                        BridgeFailure.INVALID_REQUEST,
                        false);
        failureBuild[0] = 99;
        failureContract[0] = 99;
        assertEquals(21, failure.getReportingBuildIdSha256()[0]);
        assertEquals(22, failure.getReportingContractSha256()[0]);
        byte[] failureOutput = failure.getReportingBuildIdSha256();
        failureOutput[0] = 100;
        assertEquals(21, failure.getReportingBuildIdSha256()[0]);

        byte[] identityInput = bytes(32, 23);
        BridgeIdentityObservation identity =
                new BridgeIdentityObservation(
                        context,
                        1,
                        1,
                        1,
                        identityInput,
                        identityInput,
                        identityInput,
                        identityInput,
                        identityInput,
                        identityInput,
                        identityInput,
                        identityInput,
                        identityInput,
                        1350403L,
                        "13.5.4.3",
                        identityInput,
                        identityInput,
                        13,
                        6,
                        2000,
                        2000,
                        true,
                        true);
        identityInput[0] = 99;
        assertEquals(23, identity.getCliBuildIdSha256()[0]);
        assertEquals(23, identity.getManagerPolicySha256()[0]);
        assertEquals(23, identity.getManagerCurrentSignerSha256()[0]);
        byte[] identityOutput = identity.getInstallGeneration();
        identityOutput[0] = 100;
        assertEquals(23, identity.getInstallGeneration()[0]);
    }

    @Test
    public void identityIsClosedBoundedAndDefensivelyCopied() {
        BridgeIdentityObservation observation = identity();
        assertEquals(13, observation.getShizukuApiVersion());
        assertEquals(6, observation.getShizukuServerPatchVersion());
        assertEquals(2000, observation.getReportedServerUid());
        assertEquals(2000, observation.getObservedUserServiceUid());
        assertEquals("13.5.4.3", observation.getManagerVersionName());
        assertTrue(observation.isPermissionGranted());
        assertTrue(observation.isShizukuBinderAlive());
        byte[] first = observation.getInstallGeneration();
        first[0] = 99;
        assertEquals(11, observation.getInstallGeneration()[0]);
    }

    @Test
    public void identityRejectsUnboundedOrNonAsciiVersionNameAndInvalidUid() {
        assertThrows(IllegalArgumentException.class, () -> identityWithVersionName(""));
        assertThrows(
                IllegalArgumentException.class,
                () -> identityWithVersionName("v".repeat(65)));
        assertThrows(
                IllegalArgumentException.class,
                () -> identityWithVersionName("13 release"));
        assertThrows(
                IllegalArgumentException.class,
                () -> identityWithUids(-1, 2000));
    }

    @Test
    public void systemFeaturesResultIsStructurallyInertUntilStageFive() {
        SystemFeaturesResult first = SystemFeaturesResult.inertMarker();
        SystemFeaturesResult second = SystemFeaturesResult.inertMarker();
        assertTrue(first == second);
        assertEquals(0, first.describeContents());
    }

    private static BridgeCallContext context(int version, long requestId) {
        return new BridgeCallContext(
                version,
                requestId,
                bytes(32, 1),
                bytes(32, 2),
                bytes(32, 3),
                bytes(32, 4));
    }

    private static BridgeIdentityObservation identity() {
        return identityWithUids(2000, 2000);
    }

    private static BridgeIdentityObservation identityWithVersionName(String versionName) {
        return identity(versionName, 2000, 2000);
    }

    private static BridgeIdentityObservation identityWithUids(int serverUid, int serviceUid) {
        return identity("13.5.4.3", serverUid, serviceUid);
    }

    private static BridgeIdentityObservation identity(
            String versionName, int serverUid, int serviceUid) {
        return new BridgeIdentityObservation(
                context(1, 44L),
                1,
                1,
                1,
                bytes(32, 1),
                bytes(32, 2),
                bytes(32, 3),
                bytes(32, 4),
                bytes(32, 5),
                bytes(32, 6),
                bytes(32, 7),
                bytes(32, 11),
                bytes(32, 12),
                1350403L,
                versionName,
                bytes(32, 13),
                bytes(32, 14),
                13,
                6,
                serverUid,
                serviceUid,
                true,
                true);
    }

    private static byte[] bytes(int length, int value) {
        byte[] result = new byte[length];
        java.util.Arrays.fill(result, (byte) value);
        return result;
    }
}
