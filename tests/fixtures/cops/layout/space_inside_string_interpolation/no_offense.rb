x = "hello #{name}"
y = "val: #{value}"
z = "a #{b} c"
w = "no interpolation"
v = "#{foo}#{bar}"
u = 'single quote string'
expect { cocoa_pods.current_packages }.to raise_error("Found a Podfile but no Pods directory in \
#{project_path}. Try running pod install before running license_finder.")
