x = ("hello")
    ^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a literal.

x = (1)
    ^^^ Style/RedundantParentheses: Don't use parentheses around a literal.

x = (nil)
    ^^^^^ Style/RedundantParentheses: Don't use parentheses around a literal.

x = (self)
    ^^^^^^ Style/RedundantParentheses: Don't use parentheses around a keyword.

y = (a && b)
    ^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a logical expression.

return (foo.bar)
       ^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.

x = (foo.bar)
    ^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.

x = (foo.bar(1))
    ^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.

if (arr[0])
   ^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.
end

(x == y)
^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a comparison expression.

(a >= b)
^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a comparison expression.

(x <=> y)
^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a comparison expression.

x =~ (%r{/\.{0,2}$})
     ^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a literal.

(-> { x })
^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around an expression.

(lambda { x })
^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around an expression.

(proc { x })
^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around an expression.

(defined?(:A))
^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a keyword.

(yield)
^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a keyword.

(yield())
^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a keyword.

(yield(1, 2))
^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a keyword.

(super)
^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a keyword.

def invoke_super
  (super())
  ^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a keyword.
end

(super(1, 2))
^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a keyword.

(x === y)
^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a comparison expression.

x.y((z))
    ^^^ Style/RedundantParentheses: Don't use parentheses around a method argument.

x.y((z + w))
    ^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method argument.

x&.y((z))
     ^^^ Style/RedundantParentheses: Don't use parentheses around a method argument.

x.y(a, (b))
       ^^^ Style/RedundantParentheses: Don't use parentheses around a method argument.

return (foo + bar)
       ^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.

(foo rescue bar)
^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a one-line rescue.

return (42)
       ^^^^ Style/RedundantParentheses: Don't use parentheses around a literal.

(!x arg)
^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a unary operation.

x.y((a..b))
    ^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method argument.

x.y((1..42))
    ^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method argument.

(0..10).send(@method, (3..7)).should be_true
                      ^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method argument.

"#{(foo)}"
   ^^^^^ Style/RedundantParentheses: Don't use parentheses around an interpolated expression.

(expression in pattern)
^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a one-line pattern matching.

(expression => pattern)
^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a one-line pattern matching.

(foo.bar).to_s
^ Style/RedundantParentheses: Don't use parentheses around a method call.

(foo.bar(1)).to_json
^ Style/RedundantParentheses: Don't use parentheses around a method call.

(foo.bar).qux
^ Style/RedundantParentheses: Don't use parentheses around a method call.

(x.y).z(arg)
^ Style/RedundantParentheses: Don't use parentheses around a method call.

!(@groups.include?(g))
 ^ Style/RedundantParentheses: Don't use parentheses around a method call.

foo.include?((port = get_port))
             ^ Style/RedundantParentheses: Don't use parentheses around a method argument.

({filename: file, content: File.read(file)}.merge(opts)).to_json
^ Style/RedundantParentheses: Don't use parentheses around a method call.

({filename: file, content: File.read(file)}.merge(opts)).to_json
^ Style/RedundantParentheses: Don't use parentheses around a method call.

return ((isprint(c)) ? 1 : 2)
        ^ Style/RedundantParentheses: Don't use parentheses around a method call.

exit_code = (@codaveri_evaluation_results.map(&:success).all? { |n| n == 1 }) ? 0 : 2
            ^ Style/RedundantParentheses: Don't use parentheses around a method call.

new_file = [(Pod::Sandbox::PathList.new(@banana_spec.defined_in_file.dirname).root + 'CoolFile.h')]
            ^ Style/RedundantParentheses: Don't use parentheses around a method call.

c = (((c & 0x03ff)) << 10 | (low & 0x03ff)) + 0x10000
      ^ Style/RedundantParentheses: Don't use parentheses around a method call.

((1 << 128)).to_s(16), # 0
 ^ Style/RedundantParentheses: Don't use parentheses around a method call.

((1 << 64)).to_s(16),
 ^ Style/RedundantParentheses: Don't use parentheses around a method call.

match(event,
  (on Finished do
  ^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method argument.
     on_finish
   end),
)

(`curl -s http://localhost:#{@port}/ | wc`).split(" ").first.to_i > 10
^ Style/RedundantParentheses: Don't use parentheses around a literal.

text = (
       ^ Style/RedundantParentheses: Don't use parentheses around a literal.
  `dbus-send --print-reply --dest=org.kde.klipper \
  /klipper org.kde.klipper.klipper.getClipboardContents | awk '#{awk_script}'`
).split("\n", -1)

next mem if (Array(@config[:exclude])).include? File.basename(fdir)
            ^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.

(`convert #{ico_template} -resize #{size}x#{size} #{FILE_FAVICO_DIR}/favicon-#{size}x#{size}.ico`)
^ Style/RedundantParentheses: Don't use parentheses around a literal.

(`convert #{ico_template} -resize #{size}x#{size} #{FILE_FAVICO_DIR}/apple-touch-icon-#{size}x#{size}.png`)
^ Style/RedundantParentheses: Don't use parentheses around a literal.

(`convert #{ico_template} -resize #{size}x#{size} #{FILE_FAVICO_DIR}/apple-touch-icon-#{size}x#{size}-precomposed.png`)
^ Style/RedundantParentheses: Don't use parentheses around a literal.

(`convert #{ico_template} -resize #{size}x#{size} #{FILE_FAVICO_DIR}/mstile-#{size}x#{size}.png`)
^ Style/RedundantParentheses: Don't use parentheses around a literal.

def third_party_cart(params = {})
  add_field("id_type", "1")
  (max_existing_line_item_id = form_fields.keys.map do |key|
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around an assignment.
    key
  end.compact.max || 0)
end

if((sekrets_argv = ENV['SEKRETS_ARGV']))
   ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around an assignment.
end

response = foo(
  body: ({ cmd: "_notify-validate" }.merge(paypal_event)).to_query,
        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.
)

while (pop_messages(queue_url, 10).length > 0);
      ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.
end

# Block argument with redundant parens around a variable
bar = 1
foo(&(bar))
     ^^^ Style/RedundantParentheses: Don't use parentheses around a variable.

# Block argument with redundant parens around a literal
m(&(:symbol))
   ^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a literal.

body: ({ cmd: "_notify-validate" }.merge(paypal_event)).to_query,
      ^ Style/RedundantParentheses: Don't use parentheses around a method call.

return (attributes[key] = value) unless Attributes::STYLES_MERGE.include?(key)
       ^^^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.

if((ENV['PL_REDIS_URL'.freeze] ||= ENV['REDIS_URL'.freeze]))
   ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around an assignment.
end

(self[field] = self[field].kind_of?(Numeric) ? (self[field] || 0) + value : value)
^ Style/RedundantParentheses: Don't use parentheses around an assignment.

body: ({ cmd: "_notify-validate" }.merge(paypal_event)).to_query,
      ^ Style/RedundantParentheses: Don't use parentheses around a method call.

[image_path, (resolve_image_options image_path, image_format, image_attrs, (({ background: true, container_size: [page_width, page_height] }.merge opts)))]
                                                                            ^ Style/RedundantParentheses: Don't use parentheses around a method call.

after{(r.quit rescue nil) if defined?(r)}
      ^ Style/RedundantParentheses: Don't use parentheses around a one-line rescue.

smc_v=matrix.inverse.diagonal.map{|ii| 1-(1.quo(ii))}
                                         ^ Style/RedundantParentheses: Don't use parentheses around a method call.

@initial_communalities=@matrix.inverse.diagonal.map{|i| 1-(1.quo(i))}
                                                          ^ Style/RedundantParentheses: Don't use parentheses around a method call.

until($stdin.gets).include?("DONE1")
     ^ Style/RedundantParentheses: Don't use parentheses around a method call.

until($stdin.gets).include?("DONE2")
     ^ Style/RedundantParentheses: Don't use parentheses around a method call.

until($stdin.gets).include?("DONE3")
     ^ Style/RedundantParentheses: Don't use parentheses around a method call.

if other
  work
elsif cond
  (x += 1)
  ^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around an assignment.
  work
end

if (obj = (obj.reload rescue nil))
          ^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a one-line rescue.

if (obj = (obj.reload rescue nil))
          ^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a one-line rescue.

if hlp = (send("hlp_#{c}") rescue nil)
         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a one-line rescue.

if value = (m[name] rescue nil)
           ^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a one-line rescue.

if client = (UNIXSocket.open(socket) rescue nil)
            ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a one-line rescue.

# Assignment inside string interpolation — Parser AST wraps interpolation in begin
"value is #{(x = compute)}"
            ^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around an assignment.

accumulator << ({ bar: element }.merge!(ORIGINAL_HASH){ |_key, left, _right| left })
               ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.

yield ("Decompressing sparse image #{item.name}"), :percent, (i * 100) / sparse.count_chunks if block_given?
      ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a literal.

var = 0
foo in { bar: ^(var) }
               ^^^^^ Style/RedundantParentheses: Don't use parentheses around a variable.

args = m ((0; 1))
          ^^^^^^ Style/RedundantParentheses: Don't use parentheses around a literal.

args = m ((0; 1)), ((2; 3))
          ^^^^^^ Style/RedundantParentheses: Don't use parentheses around a literal.
                    ^^^^^^ Style/RedundantParentheses: Don't use parentheses around a literal.

assert_eq(false, (not true))
                 ^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method argument.

(not(true)).should be_false
^ Style/RedundantParentheses: Don't use parentheses around a keyword.

if value =~ (configuration[:format])
            ^^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.

if Gem::Specification.any? { |s| (s.name == 'fastlane-plugin-trainer') && Gem::Requirement.default =~ (s.version) }
                                                                                                      ^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.

assert no_results_div.text =~ (Regexp.new(I18n.translate("bento_search.no_results")))
                              ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.

mail && Regexp.new(regexp) =~ (mail[field.to_sym])
                              ^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.

begin
  raise "raise"
rescue *(RuntimeError) => e
        ^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a constant.
  :expected
end

# Rescue body with assignment-wrapped logical expression still flags
begin
  work
rescue Exception => e
  error_class = e.class.to_s
  is_gss_error = (error_class.include?('GSSAPI') || error_class.include?('GssApi') || error_class.include?('GSS'))
                 ^ Style/RedundantParentheses: Don't use parentheses around a logical expression.
end

# Rescue body with parenthesized method call argument still flags
begin
  work
rescue Exception => e
  path = __FILE__
  raise ASSERTION_CLASS, e.message, (e.backtrace.reject { |line| File.expand_path(line).match?(/#{path}/) })
                                    ^ Style/RedundantParentheses: Don't use parentheses around a method call.
end

(((ruby_rails_versions_hash[ruby_version] || {})['rails']) || '').split(',')
 ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.

!!((@btc_txbuilder_provider_consumed_unspent_outputs ||= {})[btc_txbuilder_outpoint_id(output)])
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.

begin
  work
rescue Exception => e
  (disconnect rescue nil) if @state != CONNECTING_STATE
  ^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a one-line rescue.
end

begin
  work
ensure
  (cache.close rescue nil) if cache
  ^^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a one-line rescue.
end

if ((GitlabCli::Config[:display_results_in_pager] && !options['nopager']) || options['pager'])
   ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a logical expression.

confirmed = true
if (confirmed)
   ^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a variable.

assert_equal (+ 1.second), 1.second
             ^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.

# Multi-statement rescue bodies still flag direct expressions and predicates
begin
  work
rescue StandardError => e
  text = nil
  (foo.bar).to_s
  ^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.
end

begin
  work
rescue StandardError => e
  text = nil
  if ((a && b) || c)
     ^^^^^^^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a logical expression.
    handle
  end
end

begin
  work
rescue Errno::EFBIG
  text = nil
  raise if (retried)
           ^^^^^^^^^ Style/RedundantParentheses: Don't use parentheses around a method call.
end
