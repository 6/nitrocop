get :index, params: { user_id: 1 }, headers: { "ACCEPT" => "text/html" }
get :index
post :create, params: { name: "foo" }
put :update, params: { id: 1, name: "bar" }
delete :destroy, params: { id: 1 }
get :new, format: :json
get "/test", to: "admin/admin#test"
get :nothing, **args

def perform_request(...)
  get(:list, ...)
end

def perform_request2(**options)
  get(:list, **options)
end

def perform_request3(**)
  get(:list, **)
end

routes do
  get :list, on: :collection
end

Rails.application.routes.draw do
  get :list, on: :collection
end
