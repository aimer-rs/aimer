#!/usr/bin/env python3
"""Compile the production UIKit host and run routing regressions on an already-booted simulator.

Usage: python3 scripts/test_ios_hardware_input.py SIMULATOR_UDID [HOST_MAIN_SWIFT]
No simulator is created, booted, or shut down. The test app exits on success.
"""
import pathlib
import platform
import plistlib
import shutil
import subprocess
import sys
import tempfile

root = pathlib.Path(__file__).resolve().parents[1]
device = sys.argv[1]
host = pathlib.Path(sys.argv[2]) if len(sys.argv) > 2 else root / "app/jaime/builds/ios/Jaime/main.swift"
bundle_id = "dev.aimers.input-regression"
sdk = subprocess.check_output(["xcrun", "--sdk", "iphonesimulator", "--show-sdk-path"], text=True).strip()
with tempfile.TemporaryDirectory(prefix="aimer-ios-input-") as temporary:
    work = pathlib.Path(temporary)
    app = work / "InputTests.app"
    app.mkdir()
    shutil.copyfile(host, work / "main.swift")
    info = {"CFBundleExecutable": "InputTests", "CFBundleIdentifier": bundle_id,
        "CFBundleName": "InputTests", "CFBundlePackageType": "APPL", "CFBundleVersion": "1",
        "CFBundleShortVersionString": "1", "MinimumOSVersion": "14.0", "UILaunchScreen": {},
        "UIApplicationSupportsIndirectInputEvents": True,
        "UIApplicationSceneManifest": {"UISceneConfigurations": {"UIWindowSceneSessionRoleApplication": [
            {"UISceneConfigurationName": "Input tests", "UISceneDelegateClassName": "AimerInputTests.InputTestScene"}]}}}
    with (app / "Info.plist").open("wb") as output:
        plistlib.dump(info, output)
    subprocess.run(["xcrun", "--sdk", "iphonesimulator", "swiftc", "-module-name", "AimerInputTests", "-target",
        platform.machine() + "-apple-ios14.0-simulator", "-sdk", sdk,
        "-module-cache-path", str(work / "modules"), str(work / "main.swift"),
        str(root / "tests/ios_hardware_input.swift"), "-o", str(app / "InputTests")], check=True)
    subprocess.run(["xcrun", "simctl", "install", device, str(app)], check=True)
    try:
        result = subprocess.run(["xcrun", "simctl", "launch", "--console", "--terminate-running-process",
            device, bundle_id], text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
    finally:
        subprocess.run(["xcrun", "simctl", "uninstall", device, bundle_id], check=False)
    print(result.stdout)
    if result.returncode != 0 or "AIMER_UIKIT_INPUT_TESTS_PASSED" not in result.stdout:
        sys.exit(1)
