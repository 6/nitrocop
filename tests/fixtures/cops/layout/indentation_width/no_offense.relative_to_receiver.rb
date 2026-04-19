# nitrocop-config: EnforcedStyleAlignWith: relative_to_receiver, IndentationStyleEnforced: tabs, IndentationConsistencyStyle: indented_internal_methods, AccessModifierIndentationStyle: outdent, EndAlignmentStyle: variable, DefEndAlignmentStyle: def
class Manager

    private :load, :available

    def load
      body
    end
end

def name
  @name ||= case resource_name
            when Proc
      resource_name.call(controller)
            when Symbol
      controller.public_send(resource_name)
            when String
      resource_name
    else
      default_name
    end
end

# RuboCop does not treat module_function as an indented_internal_methods divider.
m = Module.new do
  module_function

  def invoked_as_script?
    File.expand_path($0) == File.expand_path(__FILE__)
  end
end

# Nested operators do not make the if-body align to the outer assignment.
content = label || if ready?
                     primary_value
                   else
                     fallback_value
                   end

# Parenthesized multiline conditionals used as the last call argument are
# ignored by RuboCop's CheckAssignment path.
process((if ready?
           primary_value
         else
           fallback_value
         end))

# `if` expressions used as hash-pair values are dropped under tabs style when
# RuboCop crashes while computing the offense range.
OT.ld 'x', {
  source: if env_key
  'ENV'
  else
  (config_key ? 'config' : 'none')
  end,
}

# RuboCop skips indented_internal_methods handling for class_eval blocks nested
# inside a method body.
def wrapper(base)
  base.class_eval do
    undef :before if method_defined? :before
    def before
      body
    end

    private

    def helper
      body
    end
  end
end

# Constant assignment breaks RuboCop's macro-scope chain for arbitrary DSL
# blocks, so `private` here is not an indented_internal_methods divider.
module Authentication
  Authenticate = CommandClass.new(inputs: %i[a]) do
    def call
      body
    rescue => e
      raise e
    end

    private

    def authenticator
      body
    end
  end
end

# RuboCop does not apply indented_internal_methods handling when a class/module
# body is wrapped in a top-level rescue node.
module Outer
  module WithJson
    def json
      json!
    rescue StandardError
      nil
    end

    private

    def raw_text
      body
    end
  rescue StandardError
    []
  end
end

# Adjacent-def modifiers use `Layout/DefEndAlignment: def` as their base.
class Highlight
  private_class_method def self.parse_cgi(str)
        pairs = URI.decode_www_form(str).map { |k, v| [k.to_sym, v] }
        Hash[pairs]
      end
end

class Cli
  private_class_method def self.normalize_syntax(argv)
      out = []
      argv.each do |arg|
        case arg
        when /^(--\w+)=(.*)$/
          out << $1 << $2
        when /^(-\w)(.+)$/
          out << $1 << $2
        else
          out << arg
        end
      end
    end
end
