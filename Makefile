# HomeRoute Build System
# Usage: make all, make deploy, make test

.PHONY: server web all deploy test clean store

# Build server binary
server:
	cd crates && cargo build --release

# Build Vite React frontend
web:
	cd web && npm run build

# Full build (server + frontend)
all: server web

# Deploy (build + restart service)
deploy: all
	systemctl restart homeroute

# Run tests
test:
	cd crates && cargo test

# Build Flutter store APK (auto-increments versionCode)
SHELL := /bin/bash
store:
	@cd store_flutter && \
	CURRENT_CODE=$$(grep 'versionCode' android/app/build.gradle.kts | sed 's/[^0-9]//g') && \
	NEW_CODE=$$((CURRENT_CODE + 1)) && \
	CURRENT_NAME=$$(grep 'versionName' android/app/build.gradle.kts | sed 's/.*"\(.*\)".*/\1/') && \
	MAJOR=$$(echo "$$CURRENT_NAME" | cut -d. -f1) && \
	MINOR=$$(echo "$$CURRENT_NAME" | cut -d. -f2) && \
	NEW_NAME="$$MAJOR.$$MINOR.$$NEW_CODE" && \
	sed -i "s/versionCode = $$CURRENT_CODE/versionCode = $$NEW_CODE/" android/app/build.gradle.kts && \
	sed -i "s/versionName = \"$$CURRENT_NAME\"/versionName = \"$$NEW_NAME\"/" android/app/build.gradle.kts && \
	sed -i "s/^version: .*/version: $$NEW_NAME+$$NEW_CODE/" pubspec.yaml && \
	echo "Building store v$$NEW_NAME (code $$NEW_CODE)..." && \
	flutter build apk --release && \
	cp build/app/outputs/flutter-apk/app-release.apk /opt/homeroute/data/store/client/homeroute-store.apk && \
	APK_SIZE=$$(stat -c%s /opt/homeroute/data/store/client/homeroute-store.apk) && \
	echo "{\"version\":\"$$NEW_NAME\",\"changelog\":\"\",\"size_bytes\":$$APK_SIZE}" > /opt/homeroute/data/store/client/version.json && \
	echo "Deployed store v$$NEW_NAME → /api/store/client/apk"

# Clean build artifacts
clean:
	cd crates && cargo clean
	rm -rf web/dist
