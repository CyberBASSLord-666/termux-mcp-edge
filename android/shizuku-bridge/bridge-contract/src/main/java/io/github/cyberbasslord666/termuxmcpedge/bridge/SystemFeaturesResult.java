package io.github.cyberbasslord666.termuxmcpedge.bridge;

import android.os.Parcel;
import android.os.Parcelable;

import java.util.Objects;

/**
 * Structurally inert Stage-2 placeholder.
 *
 * <p>The reviewed named boolean surface is deliberately deferred to Stage 5. This parcelable
 * cannot represent a feature result and no Stage-2 AIDL method can send one.</p>
 */
public final class SystemFeaturesResult implements Parcelable {
    private static final int INERT_PARCEL_VERSION = 0;
    private static final SystemFeaturesResult INSTANCE = new SystemFeaturesResult();

    private SystemFeaturesResult() {
    }

    public static SystemFeaturesResult inertMarker() {
        return INSTANCE;
    }

    private SystemFeaturesResult(Parcel source) {
        Objects.requireNonNull(source, "source");
        BridgeParcelCodec.ReadFrame frame = BridgeParcelCodec.beginReadFrame(source);
        BridgeParcelCodec.requireParcelVersion(
                BridgeParcelCodec.readInt(source, frame), INERT_PARCEL_VERSION);
        BridgeParcelCodec.finishReadFrame(source, frame);
    }

    @Override
    public int describeContents() {
        return 0;
    }

    @Override
    public void writeToParcel(Parcel destination, int flags) {
        Objects.requireNonNull(destination, "destination");
        int frameStart = BridgeParcelCodec.beginWriteFrame(destination);
        destination.writeInt(INERT_PARCEL_VERSION);
        BridgeParcelCodec.finishWriteFrame(destination, frameStart);
    }

    public static final Parcelable.Creator<SystemFeaturesResult> CREATOR =
            new Parcelable.Creator<SystemFeaturesResult>() {
                @Override
                public SystemFeaturesResult createFromParcel(Parcel source) {
                    return new SystemFeaturesResult(Objects.requireNonNull(source, "source"));
                }

                @Override
                public SystemFeaturesResult[] newArray(int size) {
                    return new SystemFeaturesResult[
                            BridgeParcelCodec.requireArraySize(size)];
                }
            };
}
