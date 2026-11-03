#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
wrapper="$root/android/gradlew"

if [ ! -f "$wrapper" ]; then
    echo "Android application is not implemented yet: android/gradlew is missing." >&2
    echo "An Android project and mobile playback backend are needed before an APK can be built." >&2
    exit 1
fi

cd "$root/android"
exec sh "$wrapper" assembleRelease "$@"
