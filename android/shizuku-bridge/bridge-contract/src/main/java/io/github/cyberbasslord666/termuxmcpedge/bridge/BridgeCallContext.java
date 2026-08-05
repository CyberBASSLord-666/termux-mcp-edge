package io.github.cyberbasslord666.termuxmcpedge.bridge;

import android.os.Parcel;
import android.os.Parcelable;

import java.util.Objects;

public final class BridgeCallContext implements Parcelable {
    public static final int PROTOCOL_VERSION = 1;
    private static final int PARCEL_VERSION = 1;

    private final int protocolVersion;
    private final long requestId;
    private final byte[] requestNonce;
    private final byte[] signedManifestSha256;
    private final byte[] rustBuildIdSha256;
    private final byte[] aidlContractSha256;

    public BridgeCallContext(
            int protocolVersion,
            long requestId,
            byte[] requestNonce,
            byte[] signedManifestSha256,
            byte[] rustBuildIdSha256,
            byte[] aidlContractSha256) {
        BridgeParcelCodec.requireProtocolVersion(protocolVersion);
        this.protocolVersion = protocolVersion;
        this.requestId = BridgeParcelCodec.requireRequestId(requestId);
        this.requestNonce = BridgeParcelCodec.requireFixed(
                requestNonce, BridgeParcelCodec.NONCE_BYTES);
        this.signedManifestSha256 = BridgeParcelCodec.requireFixed(
                signedManifestSha256, BridgeParcelCodec.SHA256_BYTES);
        this.rustBuildIdSha256 = BridgeParcelCodec.requireFixed(
                rustBuildIdSha256, BridgeParcelCodec.SHA256_BYTES);
        this.aidlContractSha256 = BridgeParcelCodec.requireFixed(
                aidlContractSha256, BridgeParcelCodec.SHA256_BYTES);
    }

    private BridgeCallContext(Parcel source) {
        this(source, null);
    }

    private BridgeCallContext(
            Parcel source, BridgeParcelCodec.ReadFrame parentFrame) {
        Objects.requireNonNull(source, "source");
        BridgeParcelCodec.ReadFrame frame = parentFrame == null
                ? BridgeParcelCodec.beginReadFrame(source)
                : BridgeParcelCodec.beginNestedReadFrame(source, parentFrame);
        BridgeParcelCodec.requireParcelVersion(
                BridgeParcelCodec.readInt(source, frame), PARCEL_VERSION);
        int decodedProtocolVersion = BridgeParcelCodec.readInt(source, frame);
        BridgeParcelCodec.requireProtocolVersion(decodedProtocolVersion);
        long decodedRequestId = BridgeParcelCodec.requireRequestId(
                BridgeParcelCodec.readLong(source, frame));
        byte[] decodedRequestNonce = BridgeParcelCodec.readFixed(
                source, frame, BridgeParcelCodec.NONCE_BYTES);
        byte[] decodedSignedManifestSha256 = BridgeParcelCodec.readFixed(
                source, frame, BridgeParcelCodec.SHA256_BYTES);
        byte[] decodedRustBuildIdSha256 = BridgeParcelCodec.readFixed(
                source, frame, BridgeParcelCodec.SHA256_BYTES);
        byte[] decodedAidlContractSha256 = BridgeParcelCodec.readFixed(
                source, frame, BridgeParcelCodec.SHA256_BYTES);
        BridgeParcelCodec.finishReadFrame(source, frame);

        this.protocolVersion = decodedProtocolVersion;
        this.requestId = decodedRequestId;
        this.requestNonce = decodedRequestNonce;
        this.signedManifestSha256 = decodedSignedManifestSha256;
        this.rustBuildIdSha256 = decodedRustBuildIdSha256;
        this.aidlContractSha256 = decodedAidlContractSha256;
    }

    static BridgeCallContext readNested(
            Parcel source, BridgeParcelCodec.ReadFrame parentFrame) {
        return new BridgeCallContext(source, Objects.requireNonNull(parentFrame, "parentFrame"));
    }

    public int getProtocolVersion() {
        return protocolVersion;
    }

    public long getRequestId() {
        return requestId;
    }

    public byte[] getRequestNonce() {
        return BridgeParcelCodec.copy(requestNonce);
    }

    public byte[] getSignedManifestSha256() {
        return BridgeParcelCodec.copy(signedManifestSha256);
    }

    public byte[] getRustBuildIdSha256() {
        return BridgeParcelCodec.copy(rustBuildIdSha256);
    }

    public byte[] getAidlContractSha256() {
        return BridgeParcelCodec.copy(aidlContractSha256);
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
        destination.writeInt(protocolVersion);
        destination.writeLong(requestId);
        BridgeParcelCodec.writeFixed(destination, requestNonce);
        BridgeParcelCodec.writeFixed(destination, signedManifestSha256);
        BridgeParcelCodec.writeFixed(destination, rustBuildIdSha256);
        BridgeParcelCodec.writeFixed(destination, aidlContractSha256);
        BridgeParcelCodec.finishWriteFrame(destination, frameStart);
    }

    public static final Parcelable.Creator<BridgeCallContext> CREATOR =
            new Parcelable.Creator<BridgeCallContext>() {
                @Override
                public BridgeCallContext createFromParcel(Parcel source) {
                    return new BridgeCallContext(Objects.requireNonNull(source, "source"));
                }

                @Override
                public BridgeCallContext[] newArray(int size) {
                    return new BridgeCallContext[BridgeParcelCodec.requireArraySize(size)];
                }
            };
}
