# nitrocop-config: EnforcedStyle: indented
def build_command
  cmd = \
    "SCARPE_DISPLAY_SERVICE=#{display_service} " +
    "SCARPE_HTML_RENDERER=#{html_renderer} " +
    "SCARPE_LOG_CONFIG=\"#{scarpe_log_config}\" " +
    "SCARPE_SSPEC_TIMEOUT=\"#{timeout}\" " +
    "#{wait_after_test ? "SCARPE_SSPEC_TIMEOUT_WAIT_AFTER_TEST=Y" : ""} " +
    "SHOES_MINITEST_EXPORT_FILE=\"#{test_output}\" " +
    "SHOES_MINITEST_CLASS_NAME=\"#{test_class_name}\" " +
    "SHOES_MINITEST_METHOD_NAME=\"#{test_method_name}\" " +
    "LOCALAPPDATA=\"#{Dir.tmpdir}\"" +
    "ruby #{SCARPE_EXE} --debug --dev #{filename}"
end
