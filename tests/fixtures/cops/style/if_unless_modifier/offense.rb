if x
^^ Style/IfUnlessModifier: Favor modifier `if` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.
  do_something
end

unless x
^^^^^^ Style/IfUnlessModifier: Favor modifier `unless` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.
  do_something
end

if condition
^^ Style/IfUnlessModifier: Favor modifier `if` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.
  foo
end

unless finished?
^^^^^^ Style/IfUnlessModifier: Favor modifier `unless` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.
  retry
end

# Parenthesized condition (non-assignment) should still be flagged
if (x > 0)
^^ Style/IfUnlessModifier: Favor modifier `if` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.
  do_something
end

# Blank line between condition and body should still be flagged
if condition
^^ Style/IfUnlessModifier: Favor modifier `if` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.

  do_something
end

# Short comment on condition line should still be flagged
if condition # short comment
^^ Style/IfUnlessModifier: Favor modifier `if` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.
  do_something
end

# One-line form should be flagged
if foo; bar; end
^^ Style/IfUnlessModifier: Favor modifier `if` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.

raise 'ERROR: BDBA Scan Failed - Check BDBA Logs for More Info...' if scan_progress_resp[:products].any? { |p| p[:status] == 'F' }
^ Style/IfUnlessModifier: Modifier form of `if` makes the line too long.

raise "ERROR: Failed to import OpenAPI/Swagger spec #{openapi_spec} into Burp Suite Pro's Sitemap." if json_sitemap.nil? || json_sitemap.empty?
^ Style/IfUnlessModifier: Modifier form of `if` makes the line too long.

raise 'ERROR: Flags --include-response-codes and --exclude-response-codes cannot be used together.' if include_http_response_codes && exclude_http_response_codes
^ Style/IfUnlessModifier: Modifier form of `if` makes the line too long.

additional_http_headers = JSON.parse(additional_http_headers, symbolize_names: true) if additional_http_headers.is_a?(String)
^ Style/IfUnlessModifier: Modifier form of `if` makes the line too long.

raise 'ERROR: Jira Server Hash not found in PWN::Env.  Run i`pwn -Y default.yaml`, then `PWN::Env` for usage.' if engine.nil?
^ Style/IfUnlessModifier: Modifier form of `if` makes the line too long.

raise 'ERROR: Jira Server Hash not found in PWN::Env.  Run i`pwn -Y default.yaml`, then `PWN::Env` for usage.' if blockchain.nil?
^ Style/IfUnlessModifier: Modifier form of `if` makes the line too long.

@@logger.warn("Omitting unlicensed fields: #{unlicensed_field_keys.join(', ')} (attempt #{create_attempts}/#{max_create_attempts}). Retrying issue creation.") if defined?(@@logger)
^ Style/IfUnlessModifier: Modifier form of `if` makes the line too long.

unless defined?(JRUBY_VERSION)
^^^^^^ Style/IfUnlessModifier: Favor modifier `unless` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.
  s.add_runtime_dependency 'oj', '>= 2.12'
end

@@import_swt_packages = DEFAULT_IMPORT_SWT_PACKAGES if !defined?(@@import_swt_packages) || (defined?(@@import_swt_packages) && @@import_swt_packages == true)
^ Style/IfUnlessModifier: Modifier form of `if` makes the line too long.

unless defined? @@logger_type
^^^^^^ Style/IfUnlessModifier: Favor modifier `unless` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.
  @@logger_type = :logger
end

unless defined? @@logging_devices
^^^^^^ Style/IfUnlessModifier: Favor modifier `unless` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.
  @@logging_devices = [:stdout, :syslog]
end

@@logging_device_file_options = {size: 1_000_000, age: 'daily', roll_by: 'number'} unless defined? @@logging_device_file_options
^ Style/IfUnlessModifier: Modifier form of `unless` makes the line too long.

if(/foo/ =~ bar)
^^ Style/IfUnlessModifier: Favor modifier `if` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.
  baz
end

if /#{foo}/ =~ bar
^^ Style/IfUnlessModifier: Favor modifier `if` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.
  baz
end

documentation_stream.puts "Processes a very descriptive generated event entry for {#{fullname}::#{evt_type}} in the API reference." unless /Process.*\s(event|command)/ =~ evh_docstr
^ Style/IfUnlessModifier: Modifier form of `unless` makes the line too long.

after_save { if user then user.update_contribution_count end }
             ^^ Style/IfUnlessModifier: Favor modifier `if` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.

after_destroy { if user then user.update_contribution_count end }
                ^^ Style/IfUnlessModifier: Favor modifier `if` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.

options ||= unless options_or_body.is_a?(Hash)
            ^^^^^^ Style/IfUnlessModifier: Favor modifier `unless` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.
  {body: options_or_body}
end || options_or_body || {}

(if defined?(@instrumented_integrations)
 ^^ Style/IfUnlessModifier: Favor modifier `if` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.
  @instrumented_integrations&.dup
end || {}).freeze

if @page && @page.to_i>10 && !current_user
^ Style/IfUnlessModifier: Favor modifier `if` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.
  @error = "未登录状态只能查看100条记录，登录后可查看1000条记录！";
end

result = "#{property.name}: #{property.type.to_s}#{if has_default_value then " = " + "#{default_value}" end}"
                                                   ^^ Style/IfUnlessModifier: Favor modifier `if` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.

unless current_user.partner? || current_user.admin?
^^^^^^ Style/IfUnlessModifier: Favor modifier `unless` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.
  redirect_to admin_root_path, alert: 'ここから先は管理者限定です！'
end

if data["debug_mode_p"]
^ Style/IfUnlessModifier: Favor modifier `if` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`.
  track(data, subject: "指手受信", body: "OK → #{data['to_user_name'].inspect}", emoji: ":OK:")
end
