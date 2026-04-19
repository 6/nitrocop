# nitrocop-config: EnforcedStyle: numeric
assert_response :content_too_large
                ^^^^^^^^^^^^^^^^^^ Rails/HttpStatus: Prefer `413` over `:content_too_large` to define HTTP status code.
head :content_too_large
     ^^^^^^^^^^^^^^^^^^ Rails/HttpStatus: Prefer `413` over `:content_too_large` to define HTTP status code.
render status: :content_too_large
               ^^^^^^^^^^^^^^^^^^ Rails/HttpStatus: Prefer `413` over `:content_too_large` to define HTTP status code.
