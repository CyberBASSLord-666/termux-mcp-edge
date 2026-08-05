package io.github.cyberbasslord666.termuxmcpedge.bridge;

import android.os.Parcel;
import android.os.Parcelable;

public final class BridgeParcelInstrumentationTest {
    public void testContextRoundTripPreservesOnlyFixedFields() {
        byte[] nonce = bytes(1);
        byte[] manifest = bytes(2);
        byte[] build = bytes(3);
        byte[] contract = bytes(4);
        BridgeCallContext expected =
                new BridgeCallContext(1, 77L, nonce, manifest, build, contract);
        nonce[0] = 99;
        BridgeCallContext actual = roundTrip(expected, BridgeCallContext.CREATOR);
        BridgeTestAssertions.equalsInt(1, actual.getProtocolVersion());
        BridgeTestAssertions.equalsLong(77L, actual.getRequestId());
        assertFixed(bytes(1), actual.getRequestNonce());
        assertFixed(manifest, actual.getSignedManifestSha256());
        assertFixed(build, actual.getRustBuildIdSha256());
        assertFixed(contract, actual.getAidlContractSha256());
        assertDefensiveCopy(actual.getRequestNonce(), actual.getRequestNonce());

        byte[] failureBuild = bytes(5);
        byte[] failureContract = bytes(6);
        BridgeFailure expectedFailure =
                new BridgeFailure(
                        expected,
                        failureBuild,
                        failureContract,
                        BridgeFailure.LIFECYCLE_CHANGED,
                        true);
        failureBuild[0] = 99;
        BridgeFailure actualFailure = roundTrip(expectedFailure, BridgeFailure.CREATOR);
        BridgeTestAssertions.equalsLong(77L, actualFailure.getContext().getRequestId());
        BridgeTestAssertions.equalsInt(
                BridgeFailure.LIFECYCLE_CHANGED, actualFailure.getFailureCode());
        BridgeTestAssertions.isTrue(actualFailure.isRetryable());
        assertFixed(bytes(5), actualFailure.getReportingBuildIdSha256());
        assertFixed(failureContract, actualFailure.getReportingContractSha256());
        assertDefensiveCopy(
                actualFailure.getReportingBuildIdSha256(),
                actualFailure.getReportingBuildIdSha256());

        BridgeIdentityObservation expectedIdentity = identity(expected);
        BridgeIdentityObservation actualIdentity =
                roundTrip(expectedIdentity, BridgeIdentityObservation.CREATOR);
        BridgeTestAssertions.equalsLong(77L, actualIdentity.getContext().getRequestId());
        BridgeTestAssertions.equalsInt(1, actualIdentity.getCliProtocolVersion());
        BridgeTestAssertions.equalsInt(1, actualIdentity.getBrokerProtocolVersion());
        BridgeTestAssertions.equalsInt(1, actualIdentity.getPrivilegedProtocolVersion());
        BridgeTestAssertions.equalsLong(1350403L, actualIdentity.getManagerVersionCode());
        BridgeTestAssertions.equalsString(
                "13.5.4.3", actualIdentity.getManagerVersionName());
        BridgeTestAssertions.equalsInt(13, actualIdentity.getShizukuApiVersion());
        BridgeTestAssertions.equalsInt(6, actualIdentity.getShizukuServerPatchVersion());
        BridgeTestAssertions.equalsInt(2000, actualIdentity.getReportedServerUid());
        BridgeTestAssertions.equalsInt(2001, actualIdentity.getObservedUserServiceUid());
        BridgeTestAssertions.isTrue(actualIdentity.isPermissionGranted());
        BridgeTestAssertions.isFalse(actualIdentity.isShizukuBinderAlive());
        assertFixed(bytes(10), actualIdentity.getCliBuildIdSha256());
        assertFixed(bytes(11), actualIdentity.getBrokerBuildIdSha256());
        assertFixed(bytes(12), actualIdentity.getPrivilegedBuildIdSha256());
        assertFixed(bytes(13), actualIdentity.getCliAidlContractSha256());
        assertFixed(bytes(14), actualIdentity.getBrokerAidlContractSha256());
        assertFixed(bytes(15), actualIdentity.getPrivilegedAidlContractSha256());
        assertFixed(bytes(16), actualIdentity.getManagerPolicySha256());
        assertFixed(bytes(17), actualIdentity.getInstallGeneration());
        assertFixed(bytes(18), actualIdentity.getServiceInstanceNonce());
        assertFixed(bytes(19), actualIdentity.getManagerBaseApkSha256());
        assertFixed(bytes(20), actualIdentity.getManagerCurrentSignerSha256());
        assertDefensiveCopy(
                actualIdentity.getManagerPolicySha256(),
                actualIdentity.getManagerPolicySha256());

        SystemFeaturesResult actualFeatures =
                roundTrip(
                        SystemFeaturesResult.inertMarker(),
                        SystemFeaturesResult.CREATOR);
        BridgeTestAssertions.equalsInt(0, actualFeatures.describeContents());
    }

    public void testUnknownContextVersionFailsClosedDuringUnparcel() {
        BridgeCallContext context = context();
        BridgeFailure failure =
                new BridgeFailure(
                        context,
                        bytes(5),
                        bytes(6),
                        BridgeFailure.IDENTITY_MISMATCH,
                        false);
        BridgeIdentityObservation identity = identity(context);
        SystemFeaturesResult features = SystemFeaturesResult.inertMarker();

        assertEveryTruncationRejected(context, BridgeCallContext.CREATOR);
        assertEveryTruncationRejected(failure, BridgeFailure.CREATOR);
        assertEveryTruncationRejected(identity, BridgeIdentityObservation.CREATOR);
        assertEveryTruncationRejected(features, SystemFeaturesResult.CREATOR);

        assertIntMutationRejected(context, BridgeCallContext.CREATOR, 4, 2);
        assertIntMutationRejected(failure, BridgeFailure.CREATOR, 4, 2);
        assertIntMutationRejected(identity, BridgeIdentityObservation.CREATOR, 4, 2);
        assertIntMutationRejected(features, SystemFeaturesResult.CREATOR, 4, 1);
        assertIntMutationRejected(context, BridgeCallContext.CREATOR, 8, 2);
        assertIntMutationRejected(context, BridgeCallContext.CREATOR, 20, 31);
        assertIdentityProtocolVersionsRejected(identity);
        assertNonCanonicalAsciiWordsRejected(identity);

        assertIntFromFrameEndRejected(failure, BridgeFailure.CREATOR, 8, 0);
        assertIntFromFrameEndRejected(failure, BridgeFailure.CREATOR, 4, 2);
        assertIntFromFrameEndRejected(identity, BridgeIdentityObservation.CREATOR, 8, 2);
        assertIntFromFrameEndRejected(identity, BridgeIdentityObservation.CREATOR, 4, 2);

        assertNestedContextCannotEscapeParent(failure, BridgeFailure.CREATOR);
        assertNestedContextCannotEscapeParent(identity, BridgeIdentityObservation.CREATOR);
        assertNestedContextSizeRejected(failure, BridgeFailure.CREATOR, -4);
        assertNestedContextSizeRejected(failure, BridgeFailure.CREATOR, 4);
        assertNestedContextSizeRejected(identity, BridgeIdentityObservation.CREATOR, -4);
        assertNestedContextSizeRejected(identity, BridgeIdentityObservation.CREATOR, 4);

        assertFrameHeaderRejected(context, BridgeCallContext.CREATOR, 0);
        assertFrameHeaderRejected(context, BridgeCallContext.CREATOR, -8);
        assertFrameHeaderRejected(context, BridgeCallContext.CREATOR, 4);
        assertFrameHeaderRejected(context, BridgeCallContext.CREATOR, 10);
        assertFrameHeaderRejected(context, BridgeCallContext.CREATOR, 4100);
        assertFrameHeaderRejected(
                context, BridgeCallContext.CREATOR, Integer.MAX_VALUE - 3);

        assertDeclaredTrailingFieldRejected(context, BridgeCallContext.CREATOR);
        assertDeclaredTrailingFieldRejected(failure, BridgeFailure.CREATOR);
        assertDeclaredTrailingFieldRejected(identity, BridgeIdentityObservation.CREATOR);
        assertDeclaredTrailingFieldRejected(features, SystemFeaturesResult.CREATOR);

        assertExactTopLevelCallerRejectsTrailing(context, BridgeCallContext.CREATOR);
        assertExactTopLevelCallerRejectsTrailing(failure, BridgeFailure.CREATOR);
        assertExactTopLevelCallerRejectsTrailing(identity, BridgeIdentityObservation.CREATOR);
        assertExactTopLevelCallerRejectsTrailing(features, SystemFeaturesResult.CREATOR);
    }

    private static BridgeCallContext context() {
        return new BridgeCallContext(
                1,
                77L,
                bytes(1),
                bytes(2),
                bytes(3),
                bytes(4));
    }

    private static BridgeIdentityObservation identity(BridgeCallContext context) {
        return new BridgeIdentityObservation(
                context,
                1,
                1,
                1,
                bytes(10),
                bytes(11),
                bytes(12),
                bytes(13),
                bytes(14),
                bytes(15),
                bytes(16),
                bytes(17),
                bytes(18),
                1350403L,
                "13.5.4.3",
                bytes(19),
                bytes(20),
                13,
                6,
                2000,
                2001,
                true,
                false);
    }

    private static <T extends Parcelable> T roundTrip(
            T value, Parcelable.Creator<T> creator) {
        Parcel parcel = encode(value);
        try {
            parcel.setDataPosition(0);
            T actual = creator.createFromParcel(parcel);
            BridgeTestAssertions.equalsInt(0, parcel.dataAvail());
            return actual;
        } finally {
            parcel.recycle();
        }
    }

    private static Parcel encode(Parcelable value) {
        Parcel parcel = Parcel.obtain();
        value.writeToParcel(parcel, 0);
        return parcel;
    }

    private static void assertEveryTruncationRejected(
            Parcelable value, Parcelable.Creator<?> creator) {
        Parcel complete = encode(value);
        byte[] encoded;
        try {
            encoded = complete.marshall();
        } finally {
            complete.recycle();
        }
        for (int cut = 0; cut < encoded.length; cut++) {
            Parcel truncated = Parcel.obtain();
            boolean rejected = false;
            try {
                try {
                    truncated.unmarshall(encoded, 0, cut);
                    truncated.setDataPosition(0);
                    creator.createFromParcel(truncated);
                } catch (RuntimeException expected) {
                    rejected = true;
                }
                if (!rejected) {
                    throw new AssertionError(
                            "truncated parcel accepted at byte " + cut + " of " + encoded.length);
                }
            } finally {
                truncated.recycle();
            }
        }
    }

    private static void assertIntMutationRejected(
            Parcelable value,
            Parcelable.Creator<?> creator,
            int byteOffset,
            int replacement) {
        Parcel parcel = encode(value);
        try {
            parcel.setDataPosition(byteOffset);
            parcel.writeInt(replacement);
            assertCreatorRejected(parcel, creator);
        } finally {
            parcel.recycle();
        }
    }

    private static void assertIntFromFrameEndRejected(
            Parcelable value,
            Parcelable.Creator<?> creator,
            int bytesBeforeEnd,
            int replacement) {
        Parcel parcel = encode(value);
        try {
            parcel.setDataPosition(0);
            int frameSize = parcel.readInt();
            parcel.setDataPosition(frameSize - bytesBeforeEnd);
            parcel.writeInt(replacement);
            assertCreatorRejected(parcel, creator);
        } finally {
            parcel.recycle();
        }
    }

    private static void assertNestedContextCannotEscapeParent(
            Parcelable value, Parcelable.Creator<?> creator) {
        Parcel parcel = encode(value);
        try {
            parcel.setDataPosition(0);
            int outerFrameSize = parcel.readInt();
            parcel.setDataPosition(8);
            parcel.writeInt(outerFrameSize);
            assertCreatorRejected(parcel, creator);
        } finally {
            parcel.recycle();
        }
    }

    private static void assertNestedContextSizeRejected(
            Parcelable value, Parcelable.Creator<?> creator, int sizeDelta) {
        Parcel parcel = encode(value);
        try {
            parcel.setDataPosition(8);
            int nestedFrameSize = parcel.readInt();
            parcel.setDataPosition(8);
            parcel.writeInt(nestedFrameSize + sizeDelta);
            assertCreatorRejected(parcel, creator);
        } finally {
            parcel.recycle();
        }
    }

    private static void assertIdentityProtocolVersionsRejected(
            BridgeIdentityObservation value) {
        Parcel layout = encode(value);
        int firstProtocolOffset;
        try {
            layout.setDataPosition(8);
            firstProtocolOffset = 8 + layout.readInt();
        } finally {
            layout.recycle();
        }
        assertIntMutationRejected(
                value, BridgeIdentityObservation.CREATOR, firstProtocolOffset, 2);
        assertIntMutationRejected(
                value,
                BridgeIdentityObservation.CREATOR,
                firstProtocolOffset + Integer.BYTES,
                2);
        assertIntMutationRejected(
                value,
                BridgeIdentityObservation.CREATOR,
                firstProtocolOffset + Integer.BYTES * 2,
                2);
    }

    private static void assertNonCanonicalAsciiWordsRejected(
            BridgeIdentityObservation value) {
        Parcel layout = encode(value);
        int firstTokenWordOffset;
        try {
            layout.setDataPosition(8);
            int nestedFrameSize = layout.readInt();
            layout.setDataPosition(8 + nestedFrameSize);
            layout.readInt();
            layout.readInt();
            layout.readInt();
            byte[] fixed = new byte[32];
            for (int index = 0; index < 9; index++) {
                layout.readByteArray(fixed);
            }
            layout.readLong();
            BridgeTestAssertions.equalsInt(8, layout.readInt());
            firstTokenWordOffset = layout.dataPosition();
        } finally {
            layout.recycle();
        }
        assertIntMutationRejected(
                value,
                BridgeIdentityObservation.CREATOR,
                firstTokenWordOffset,
                0x00000141);
        assertIntMutationRejected(
                value,
                BridgeIdentityObservation.CREATOR,
                firstTokenWordOffset,
                0xffffff41);
    }

    private static void assertFrameHeaderRejected(
            Parcelable value, Parcelable.Creator<?> creator, int replacement) {
        assertIntMutationRejected(value, creator, 0, replacement);
    }

    private static void assertDeclaredTrailingFieldRejected(
            Parcelable value, Parcelable.Creator<?> creator) {
        Parcel parcel = encode(value);
        try {
            parcel.setDataPosition(0);
            int frameSize = parcel.readInt();
            parcel.setDataPosition(frameSize);
            parcel.writeInt(0x5a5a5a5a);
            parcel.setDataPosition(0);
            parcel.writeInt(frameSize + Integer.BYTES);
            assertCreatorRejected(parcel, creator);
        } finally {
            parcel.recycle();
        }
    }

    private static void assertExactTopLevelCallerRejectsTrailing(
            Parcelable value, Parcelable.Creator<?> creator) {
        Parcel parcel = encode(value);
        try {
            parcel.writeInt(0x5a5a5a5a);
            parcel.setDataPosition(0);
            boolean rejected = false;
            try {
                creator.createFromParcel(parcel);
                if (parcel.dataAvail() != 0) {
                    throw new IllegalArgumentException("top-level trailing parcel data");
                }
            } catch (RuntimeException expected) {
                rejected = true;
            }
            if (!rejected) {
                throw new AssertionError("exact top-level caller accepted trailing data");
            }
        } finally {
            parcel.recycle();
        }
    }

    private static void assertCreatorRejected(
            Parcel parcel, Parcelable.Creator<?> creator) {
        parcel.setDataPosition(0);
        try {
            creator.createFromParcel(parcel);
            throw new AssertionError("malformed parcel accepted");
        } catch (RuntimeException expected) {
            // Expected: all malformed encodings fail closed without returning an object.
        }
    }

    private static void assertFixed(byte[] expected, byte[] actual) {
        BridgeTestAssertions.isTrue(java.util.Arrays.equals(expected, actual));
    }

    private static void assertDefensiveCopy(byte[] first, byte[] second) {
        BridgeTestAssertions.isTrue(first != second);
        byte original = second[0];
        first[0] = (byte) (original + 1);
        BridgeTestAssertions.equalsInt(original, second[0]);
    }

    private static byte[] bytes(int value) {
        byte[] result = new byte[32];
        java.util.Arrays.fill(result, (byte) value);
        return result;
    }
}
