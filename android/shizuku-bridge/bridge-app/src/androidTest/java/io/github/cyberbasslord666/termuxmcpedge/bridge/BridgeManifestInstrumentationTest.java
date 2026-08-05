package io.github.cyberbasslord666.termuxmcpedge.bridge;

import android.content.Context;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;

public final class BridgeManifestInstrumentationTest {
    public void testTargetManifestHasNoPermissionOrComponent(Context target) throws Exception {
        PackageInfo packageInfo =
                target.getPackageManager().getPackageInfo(
                        target.getPackageName(),
                        PackageManager.GET_PERMISSIONS
                                | PackageManager.GET_ACTIVITIES
                                | PackageManager.GET_RECEIVERS
                                | PackageManager.GET_SERVICES
                                | PackageManager.GET_PROVIDERS);
        BridgeTestAssertions.isNull(packageInfo.requestedPermissions);
        BridgeTestAssertions.isNull(packageInfo.activities);
        BridgeTestAssertions.isNull(packageInfo.receivers);
        BridgeTestAssertions.isNull(packageInfo.services);
        BridgeTestAssertions.isNull(packageInfo.providers);
        BridgeTestAssertions.isFalse(
                (packageInfo.applicationInfo.flags
                        & android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0);
        BridgeTestAssertions.isFalse(
                (packageInfo.applicationInfo.flags
                        & android.content.pm.ApplicationInfo.FLAG_ALLOW_BACKUP) != 0);
    }
}
