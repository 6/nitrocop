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
