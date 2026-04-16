# nitrocop-config: EnforcedStyleAlignWith: relative_to_receiver

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

class Manager < Base

    private :load, :available, :loaded, :load_all
    public :load, :available, :loaded, :load_all

    def load(checks)
    end
end

	if fn == "|-|"
    error = "|-| Could not find section in document, please verify."
    puts error
    slim :error
	else
		# write entry to database
		file = Oxfile.new
	end
