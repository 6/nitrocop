# nitrocop-config: EnforcedStyle: space
expect { cocoa_pods.current_packages }.to raise_error("Found a Podfile but no Pods directory in \
#{ project_path }. Try running pod install before running license_finder.")
expect { cocoa_pods.current_packages }.to raise_error("Found a Podfile but no Pods directory in \
#{project_path}. Try running pod install before running license_finder.")
