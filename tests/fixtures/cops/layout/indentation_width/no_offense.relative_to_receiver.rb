# nitrocop-config: EnforcedStyleAlignWith: relative_to_receiver, IndentationStyleEnforced: tabs, IndentationConsistencyStyle: indented_internal_methods, AccessModifierIndentationStyle: outdent
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
