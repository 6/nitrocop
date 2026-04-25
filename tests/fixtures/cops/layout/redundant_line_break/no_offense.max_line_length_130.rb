# nitrocop-config: MaxLineLength: 130

module OpenProjectSafeNavigationRegression
  class OmniauthService
    def find_existing_user(user_attributes)
      UserAuthProviderLink
        .left_joins(:principal)
        .where(principal: { type: "User" })
        .with_identity_url(user_attributes[:identity_url])
        .first
        &.principal
    end
  end

  class WorkPackageDatepicker
    def input_aria_related_element(input_element, describedby:)
      input_element["aria-describedby"]
        .split
        .find { it.start_with?("#{describedby}-") }
        &.then { |id| find(id:, visible: :all) }
    end
  end
end
