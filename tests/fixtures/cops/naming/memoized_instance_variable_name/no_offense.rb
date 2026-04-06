def foo
  @foo ||= compute
end
def bar?
  @bar ||= calculate
end
def something
  @something ||= fetch
end
def value
  @value ||= expensive
end

# Not memoization: ||= is not the sole/last statement in method body
def setting(key, options = {})
  @definitions ||= {}
  UserSettings::Setting.new(key, options)
end

def readpartial(size)
  @deadline ||= Process.clock_gettime(Process::CLOCK_MONOTONIC) + @read_deadline
  @socket.read_nonblock(size)
end

def process_url
  @card ||= PreviewCard.new(url: @url)
  attempt_oembed || attempt_opengraph
end

# defined? memoization pattern with matching name
def issue_token
  return @issue_token if defined?(@issue_token)
  @issue_token = create_token
end

# defined? with bang method (! stripped)
def compute!
  return @compute if defined?(@compute)
  @compute = heavy_calculation
end

# Setter method: = suffix stripped from method name
def max_time=(value)
  @max_time ||= value
end

def tab_color=(value)
  @tab_color ||= value
end

# Leading underscore method in disallowed style: @ivar matches method without leading _
def _rate_limit_key
  @rate_limit_key ||= compute_key
end

def _strategies
  @strategies ||= build_strategies
end

# define_method with matching ivar name
define_method(:values) do
  @values ||= do_something
end

# define_singleton_method with matching ivar name
define_singleton_method(:records) do
  @records ||= fetch_records
end

# klass.define_method with matching ivar name
klass.define_method(:items) do
  @items ||= load_items
end

# singleton method (def self.x) with matching ivar
def self.records
  @records ||= fetch_records
end

# Conditional memoization: ||= in else branch with matching name
def url_helpers
  if supports_path
    generate_url_helpers(true)
  else
    @url_helpers ||= generate_url_helpers(false)
  end
end

# unless modifier with matching name
def list_users
  @list_users ||= @user_provider.list_users unless @user_provider.nil?
end

# case/when/else with matching name
def size
  case @value
  when String
    @value.bytesize
  else
    @size ||= Marshal.dump(@value).bytesize
  end
end

# block wrapping with matching name
def ensure_subscribed
  @mutex.synchronize do
    @ensure_subscribed ||= do_subscribe
  end
end

# ensure block with matching name
def infer_decoder
  compute_decoder
rescue => e
  warn(e)
ensure
  @infer_decoder ||= :itself.to_proc
end

# ternary with matching name
def encode_language(value = nil)
  value ? @encode_language = value : @encode_language ||= 'en'
end

# NOT memoization: ||= is not the last statement in the else branch
def get_config
  if default?
    use_defaults
  else
    @cached ||= compute
    finalize(@cached)
  end
end

# NOT memoization: ||= inside block body with other statements after it
def wrapped
  mutex.synchronize do
    @cached ||= compute
    process(@cached)
  end
end

# NOT memoization: ||= inside elsif/else chain (2+ levels deep)
def nested
  if a
    x
  elsif b
    y
  else
    @cached ||= compute
  end
end

# NOT memoization: ||= inside multi-statement block (last statement of multi-stmt body)
def parsed_data
  validate_input
  handle_errors do
    @data ||= load_data
  end
end

# NOT memoization: ||= is last of multiple statements in else branch
def assign_handler(handler)
  if @_handler = handler
    @_request = handler.request if handler.respond_to?(:request)
  else
    @_request ||= nil
    @_config = {}
    @_default_handler ||= nil
  end
end

# NOT memoization: ||= in multi-statement ensure block
def run_inline(name, &block)
  @running = true
  @current ||= new_item(name)
  block.call(@current)
  @current = nil
ensure
  @running = false
  @advanced ||= @current&.advanced?
end

# NOT memoization: define_method block inside a different def should not
# be traversed from the enclosing def context
def self.build_accessor(resource)
  define_method :resource do
    @resource ||= ResourceReader.new(resource.new(id))
  end
end

# NOT memoization: ||= inside multi-statement block (FP fix for VCR.use_cassette pattern)
def group
  cassette("test") do
    @ticket ||= create_record
    @ticket ||= build_record
  end
end

# NOT memoization: ||= deeply nested inside fetch block with if/else
# Parser's body.children.last stops at the block body (if-node), not the or_asgn
def inverse_name
  options.fetch(:inverse_of) do
    if @automatic_inverse_of == false
      nil
    else
      @automatic_inverse_of ||= automatic_inverse_of
    end
  end
end

# NOT memoization: ||= inside multi-statement block within self.included
def self.included(base)
  base.setup do
    @model ||= self.class.name.match(/(.*)Test$/)[1].constantize
    @title_field ||= find_field(@model)
  end
end

# && guard with matching ||= (Parser's children.last on `and` returns right operand)
def triggerable
  trigger && @triggerable ||= compute
end

# complex && guard with begin block and matching ivar
def site
  (self.scheme || self.authority) && @site ||= begin
    "computed"
  end
end

# || guard with super, matching ivar (Parser's children.last on `or` returns right operand)
def current_user
  super || @current_user ||= compute
end

# local variable assignment wrapping ||= with matching ivar
def root
  __skip__ = @root ||= begin
    "computed"
  end
end
