# nitrocop-config: EnforcedStyle: numeric
assert_response :payload_too_large
head :payload_too_large
render status: :payload_too_large
