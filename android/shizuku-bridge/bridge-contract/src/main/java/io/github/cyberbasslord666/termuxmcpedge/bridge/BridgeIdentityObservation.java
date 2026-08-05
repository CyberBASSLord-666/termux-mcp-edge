package io.github.cyberbasslord666.termuxmcpedge.bridge;

import android.os.Parcel;
import android.os.Parcelable;

import java.util.Objects;

public final class BridgeIdentityObservation implements Parcelable {
    private static final int PARCEL_VERSION = 1;

    private final BridgeCallContext context;
    private final int cliProtocolVersion;
    private final int brokerProtocolVersion;
    private final int privilegedProtocolVersion;
    private final byte[] cliBuildIdSha256;
    private final byte[] brokerBuildIdSha256;
    private final byte[] privilegedBuildIdSha256;
    private final byte[] cliAidlContractSha256;
    private final byte[] brokerAidlContractSha256;
    private final byte[] privilegedAidlContractSha256;
    private final byte[] managerPolicySha256;
    private final byte[] installGeneration;
    private final byte[] serviceInstanceNonce;
    private final long managerVersionCode;
    private final String managerVersionName;
    private final byte[] managerBaseApkSha256;
    private final byte[] managerCurrentSignerSha256;
    private final int shizukuApiVersion;
    private final int shizukuServerPatchVersion;
    private final int reportedServerUid;
    private final int observedUserServiceUid;
    private final boolean permissionGranted;
    private final boolean shizukuBinderAlive;

    @SuppressWarnings("checkstyle:ParameterNumber")
    public BridgeIdentityObservation(
            BridgeCallContext context,
            int cliProtocolVersion,
            int brokerProtocolVersion,
            int privilegedProtocolVersion,
            byte[] cliBuildIdSha256,
            byte[] brokerBuildIdSha256,
            byte[] privilegedBuildIdSha256,
            byte[] cliAidlContractSha256,
            byte[] brokerAidlContractSha256,
            byte[] privilegedAidlContractSha256,
            byte[] managerPolicySha256,
            byte[] installGeneration,
            byte[] serviceInstanceNonce,
            long managerVersionCode,
            String managerVersionName,
            byte[] managerBaseApkSha256,
            byte[] managerCurrentSignerSha256,
            int shizukuApiVersion,
            int shizukuServerPatchVersion,
            int reportedServerUid,
            int observedUserServiceUid,
            boolean permissionGranted,
            boolean shizukuBinderAlive) {
        this.context = Objects.requireNonNull(context, "context");
        BridgeParcelCodec.requireProtocolVersion(cliProtocolVersion);
        BridgeParcelCodec.requireProtocolVersion(brokerProtocolVersion);
        BridgeParcelCodec.requireProtocolVersion(privilegedProtocolVersion);
        this.cliProtocolVersion = cliProtocolVersion;
        this.brokerProtocolVersion = brokerProtocolVersion;
        this.privilegedProtocolVersion = privilegedProtocolVersion;
        this.cliBuildIdSha256 = fixed(cliBuildIdSha256);
        this.brokerBuildIdSha256 = fixed(brokerBuildIdSha256);
        this.privilegedBuildIdSha256 = fixed(privilegedBuildIdSha256);
        this.cliAidlContractSha256 = fixed(cliAidlContractSha256);
        this.brokerAidlContractSha256 = fixed(brokerAidlContractSha256);
        this.privilegedAidlContractSha256 = fixed(privilegedAidlContractSha256);
        this.managerPolicySha256 = fixed(managerPolicySha256);
        this.installGeneration = fixed(installGeneration);
        this.serviceInstanceNonce = fixed(serviceInstanceNonce);
        if (managerVersionCode <= 0L) {
            throw new IllegalArgumentException("manager version code must be positive");
        }
        this.managerVersionCode = managerVersionCode;
        this.managerVersionName = BridgeParcelCodec.requireAsciiToken(
                managerVersionName, BridgeParcelCodec.MAX_MANAGER_VERSION_NAME_BYTES);
        this.managerBaseApkSha256 = fixed(managerBaseApkSha256);
        this.managerCurrentSignerSha256 = fixed(managerCurrentSignerSha256);
        if (shizukuApiVersion <= 0 || shizukuServerPatchVersion < 0) {
            throw new IllegalArgumentException("invalid Shizuku API identity");
        }
        this.shizukuApiVersion = shizukuApiVersion;
        this.shizukuServerPatchVersion = shizukuServerPatchVersion;
        if (reportedServerUid < 0 || observedUserServiceUid < 0) {
            throw new IllegalArgumentException("invalid UID observation");
        }
        this.reportedServerUid = reportedServerUid;
        this.observedUserServiceUid = observedUserServiceUid;
        this.permissionGranted = permissionGranted;
        this.shizukuBinderAlive = shizukuBinderAlive;
    }

    private BridgeIdentityObservation(Parcel source) {
        Objects.requireNonNull(source, "source");
        BridgeParcelCodec.ReadFrame frame = BridgeParcelCodec.beginReadFrame(source);
        BridgeParcelCodec.requireParcelVersion(
                BridgeParcelCodec.readInt(source, frame), PARCEL_VERSION);
        BridgeCallContext decodedContext = BridgeCallContext.readNested(source, frame);

        int decodedCliProtocolVersion = BridgeParcelCodec.readInt(source, frame);
        int decodedBrokerProtocolVersion = BridgeParcelCodec.readInt(source, frame);
        int decodedPrivilegedProtocolVersion = BridgeParcelCodec.readInt(source, frame);
        BridgeParcelCodec.requireProtocolVersion(decodedCliProtocolVersion);
        BridgeParcelCodec.requireProtocolVersion(decodedBrokerProtocolVersion);
        BridgeParcelCodec.requireProtocolVersion(decodedPrivilegedProtocolVersion);

        byte[] decodedCliBuildIdSha256 = readFixed(source, frame);
        byte[] decodedBrokerBuildIdSha256 = readFixed(source, frame);
        byte[] decodedPrivilegedBuildIdSha256 = readFixed(source, frame);
        byte[] decodedCliAidlContractSha256 = readFixed(source, frame);
        byte[] decodedBrokerAidlContractSha256 = readFixed(source, frame);
        byte[] decodedPrivilegedAidlContractSha256 = readFixed(source, frame);
        byte[] decodedManagerPolicySha256 = readFixed(source, frame);
        byte[] decodedInstallGeneration = readFixed(source, frame);
        byte[] decodedServiceInstanceNonce = readFixed(source, frame);

        long decodedManagerVersionCode = BridgeParcelCodec.readLong(source, frame);
        if (decodedManagerVersionCode <= 0L) {
            throw new IllegalArgumentException("manager version code must be positive");
        }
        String decodedManagerVersionName = BridgeParcelCodec.readAsciiToken(
                source, frame, BridgeParcelCodec.MAX_MANAGER_VERSION_NAME_BYTES);
        byte[] decodedManagerBaseApkSha256 = readFixed(source, frame);
        byte[] decodedManagerCurrentSignerSha256 = readFixed(source, frame);

        int decodedShizukuApiVersion = BridgeParcelCodec.readInt(source, frame);
        int decodedShizukuServerPatchVersion = BridgeParcelCodec.readInt(source, frame);
        if (decodedShizukuApiVersion <= 0 || decodedShizukuServerPatchVersion < 0) {
            throw new IllegalArgumentException("invalid Shizuku API identity");
        }
        int decodedReportedServerUid = BridgeParcelCodec.readInt(source, frame);
        int decodedObservedUserServiceUid = BridgeParcelCodec.readInt(source, frame);
        if (decodedReportedServerUid < 0 || decodedObservedUserServiceUid < 0) {
            throw new IllegalArgumentException("invalid UID observation");
        }
        boolean decodedPermissionGranted = BridgeParcelCodec.readBoolean(source, frame);
        boolean decodedShizukuBinderAlive = BridgeParcelCodec.readBoolean(source, frame);
        BridgeParcelCodec.finishReadFrame(source, frame);

        this.context = decodedContext;
        this.cliProtocolVersion = decodedCliProtocolVersion;
        this.brokerProtocolVersion = decodedBrokerProtocolVersion;
        this.privilegedProtocolVersion = decodedPrivilegedProtocolVersion;
        this.cliBuildIdSha256 = decodedCliBuildIdSha256;
        this.brokerBuildIdSha256 = decodedBrokerBuildIdSha256;
        this.privilegedBuildIdSha256 = decodedPrivilegedBuildIdSha256;
        this.cliAidlContractSha256 = decodedCliAidlContractSha256;
        this.brokerAidlContractSha256 = decodedBrokerAidlContractSha256;
        this.privilegedAidlContractSha256 = decodedPrivilegedAidlContractSha256;
        this.managerPolicySha256 = decodedManagerPolicySha256;
        this.installGeneration = decodedInstallGeneration;
        this.serviceInstanceNonce = decodedServiceInstanceNonce;
        this.managerVersionCode = decodedManagerVersionCode;
        this.managerVersionName = decodedManagerVersionName;
        this.managerBaseApkSha256 = decodedManagerBaseApkSha256;
        this.managerCurrentSignerSha256 = decodedManagerCurrentSignerSha256;
        this.shizukuApiVersion = decodedShizukuApiVersion;
        this.shizukuServerPatchVersion = decodedShizukuServerPatchVersion;
        this.reportedServerUid = decodedReportedServerUid;
        this.observedUserServiceUid = decodedObservedUserServiceUid;
        this.permissionGranted = decodedPermissionGranted;
        this.shizukuBinderAlive = decodedShizukuBinderAlive;
    }

    private static byte[] fixed(byte[] value) {
        return BridgeParcelCodec.requireFixed(value, BridgeParcelCodec.SHA256_BYTES);
    }

    private static byte[] readFixed(
            Parcel source, BridgeParcelCodec.ReadFrame frame) {
        return BridgeParcelCodec.readFixed(
                source, frame, BridgeParcelCodec.SHA256_BYTES);
    }

    public BridgeCallContext getContext() {
        return context;
    }

    public int getCliProtocolVersion() {
        return cliProtocolVersion;
    }

    public int getBrokerProtocolVersion() {
        return brokerProtocolVersion;
    }

    public int getPrivilegedProtocolVersion() {
        return privilegedProtocolVersion;
    }

    public byte[] getCliBuildIdSha256() {
        return BridgeParcelCodec.copy(cliBuildIdSha256);
    }

    public byte[] getBrokerBuildIdSha256() {
        return BridgeParcelCodec.copy(brokerBuildIdSha256);
    }

    public byte[] getPrivilegedBuildIdSha256() {
        return BridgeParcelCodec.copy(privilegedBuildIdSha256);
    }

    public byte[] getCliAidlContractSha256() {
        return BridgeParcelCodec.copy(cliAidlContractSha256);
    }

    public byte[] getBrokerAidlContractSha256() {
        return BridgeParcelCodec.copy(brokerAidlContractSha256);
    }

    public byte[] getPrivilegedAidlContractSha256() {
        return BridgeParcelCodec.copy(privilegedAidlContractSha256);
    }

    public byte[] getManagerPolicySha256() {
        return BridgeParcelCodec.copy(managerPolicySha256);
    }

    public byte[] getInstallGeneration() {
        return BridgeParcelCodec.copy(installGeneration);
    }

    public byte[] getServiceInstanceNonce() {
        return BridgeParcelCodec.copy(serviceInstanceNonce);
    }

    public long getManagerVersionCode() {
        return managerVersionCode;
    }

    public String getManagerVersionName() {
        return managerVersionName;
    }

    public byte[] getManagerBaseApkSha256() {
        return BridgeParcelCodec.copy(managerBaseApkSha256);
    }

    public byte[] getManagerCurrentSignerSha256() {
        return BridgeParcelCodec.copy(managerCurrentSignerSha256);
    }

    public int getShizukuApiVersion() {
        return shizukuApiVersion;
    }

    public int getShizukuServerPatchVersion() {
        return shizukuServerPatchVersion;
    }

    public int getReportedServerUid() {
        return reportedServerUid;
    }

    public int getObservedUserServiceUid() {
        return observedUserServiceUid;
    }

    public boolean isPermissionGranted() {
        return permissionGranted;
    }

    public boolean isShizukuBinderAlive() {
        return shizukuBinderAlive;
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
        destination.writeInt(cliProtocolVersion);
        destination.writeInt(brokerProtocolVersion);
        destination.writeInt(privilegedProtocolVersion);
        writeFixed(destination, cliBuildIdSha256);
        writeFixed(destination, brokerBuildIdSha256);
        writeFixed(destination, privilegedBuildIdSha256);
        writeFixed(destination, cliAidlContractSha256);
        writeFixed(destination, brokerAidlContractSha256);
        writeFixed(destination, privilegedAidlContractSha256);
        writeFixed(destination, managerPolicySha256);
        writeFixed(destination, installGeneration);
        writeFixed(destination, serviceInstanceNonce);
        destination.writeLong(managerVersionCode);
        BridgeParcelCodec.writeAsciiToken(destination, managerVersionName);
        writeFixed(destination, managerBaseApkSha256);
        writeFixed(destination, managerCurrentSignerSha256);
        destination.writeInt(shizukuApiVersion);
        destination.writeInt(shizukuServerPatchVersion);
        destination.writeInt(reportedServerUid);
        destination.writeInt(observedUserServiceUid);
        destination.writeInt(permissionGranted ? 1 : 0);
        destination.writeInt(shizukuBinderAlive ? 1 : 0);
        BridgeParcelCodec.finishWriteFrame(destination, frameStart);
    }

    private static void writeFixed(Parcel destination, byte[] value) {
        BridgeParcelCodec.writeFixed(destination, value);
    }

    public static final Parcelable.Creator<BridgeIdentityObservation> CREATOR =
            new Parcelable.Creator<BridgeIdentityObservation>() {
                @Override
                public BridgeIdentityObservation createFromParcel(Parcel source) {
                    return new BridgeIdentityObservation(
                            Objects.requireNonNull(source, "source"));
                }

                @Override
                public BridgeIdentityObservation[] newArray(int size) {
                    return new BridgeIdentityObservation[
                            BridgeParcelCodec.requireArraySize(size)];
                }
            };
}
