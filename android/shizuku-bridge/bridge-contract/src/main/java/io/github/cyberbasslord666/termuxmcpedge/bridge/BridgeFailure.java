package io.github.cyberbasslord666.termuxmcpedge.bridge;

import android.os.Parcel;
import android.os.Parcelable;

import java.util.Objects;

public final class BridgeFailure implements Parcelable {
    private static final int PARCEL_VERSION = 1;

    public static final int INVALID_REQUEST = 1;
    public static final int BRIDGE_UNAVAILABLE = 2;
    public static final int PERMISSION_DENIED = 3;
    public static final int IDENTITY_MISMATCH = 4;
    public static final int LIFECYCLE_CHANGED = 5;
    public static final int CANCELLED = 6;
    public static final int TIMED_OUT = 7;
    public static final int INTERNAL_REJECTED = 8;

    private final BridgeCallContext context;
    private final byte[] reportingBuildIdSha256;
    private final byte[] reportingContractSha256;
    private final int failureCode;
    private final boolean retryable;

    public BridgeFailure(
            BridgeCallContext context,
            byte[] reportingBuildIdSha256,
            byte[] reportingContractSha256,
            int failureCode,
            boolean retryable) {
        this.context = Objects.requireNonNull(context, "context");
        this.reportingBuildIdSha256 = BridgeParcelCodec.requireFixed(
                reportingBuildIdSha256, BridgeParcelCodec.SHA256_BYTES);
        this.reportingContractSha256 = BridgeParcelCodec.requireFixed(
                reportingContractSha256, BridgeParcelCodec.SHA256_BYTES);
        this.failureCode = requireFailureCode(failureCode);
        this.retryable = retryable;
    }

    private BridgeFailure(Parcel source) {
        Objects.requireNonNull(source, "source");
        BridgeParcelCodec.ReadFrame frame = BridgeParcelCodec.beginReadFrame(source);
        BridgeParcelCodec.requireParcelVersion(
                BridgeParcelCodec.readInt(source, frame), PARCEL_VERSION);
        BridgeCallContext decodedContext = BridgeCallContext.readNested(source, frame);
        byte[] decodedReportingBuildIdSha256 = BridgeParcelCodec.readFixed(
                source, frame, BridgeParcelCodec.SHA256_BYTES);
        byte[] decodedReportingContractSha256 = BridgeParcelCodec.readFixed(
                source, frame, BridgeParcelCodec.SHA256_BYTES);
        int decodedFailureCode = requireFailureCode(
                BridgeParcelCodec.readInt(source, frame));
        boolean decodedRetryable = BridgeParcelCodec.readBoolean(source, frame);
        BridgeParcelCodec.finishReadFrame(source, frame);

        this.context = decodedContext;
        this.reportingBuildIdSha256 = decodedReportingBuildIdSha256;
        this.reportingContractSha256 = decodedReportingContractSha256;
        this.failureCode = decodedFailureCode;
        this.retryable = decodedRetryable;
    }

    private static int requireFailureCode(int failureCode) {
        if (failureCode < INVALID_REQUEST || failureCode > INTERNAL_REJECTED) {
            throw new IllegalArgumentException("unknown failure code");
        }
        return failureCode;
    }

    public BridgeCallContext getContext() {
        return context;
    }

    public byte[] getReportingBuildIdSha256() {
        return BridgeParcelCodec.copy(reportingBuildIdSha256);
    }

    public byte[] getReportingContractSha256() {
        return BridgeParcelCodec.copy(reportingContractSha256);
    }

    public int getFailureCode() {
        return failureCode;
    }

    public boolean isRetryable() {
        return retryable;
    }

    @Override
    public int describeContents() {
        return 0;
    }

    @Override
    public void writeToParcel(Parcel destination, int flags) {
        Objects.requireNonNull(destination, "destination");
        int frameStart = BridgeParcelCodec.beginWriteFrame(destination);
        destination.writeInt(PARCEL_VERSION);
        context.writeToParcel(destination, flags);
        BridgeParcelCodec.writeFixed(destination, reportingBuildIdSha256);
        BridgeParcelCodec.writeFixed(destination, reportingContractSha256);
        destination.writeInt(failureCode);
        destination.writeInt(retryable ? 1 : 0);
        BridgeParcelCodec.finishWriteFrame(destination, frameStart);
    }

    public static final Parcelable.Creator<BridgeFailure> CREATOR =
            new Parcelable.Creator<BridgeFailure>() {
                @Override
                public BridgeFailure createFromParcel(Parcel source) {
                    return new BridgeFailure(Objects.requireNonNull(source, "source"));
                }

                @Override
                public BridgeFailure[] newArray(int size) {
                    return new BridgeFailure[BridgeParcelCodec.requireArraySize(size)];
                }
            };
}
