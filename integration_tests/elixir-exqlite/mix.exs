defmodule ScytheIntegrationTest.MixProject do
  use Mix.Project

  def project do
    [
      app: :scythe_integration_test,
      version: "0.1.0",
      elixir: "~> 1.14",
      start_permanent: false,

      deps: deps()
    ]
  end

  def application do
    [
      extra_applications: [:logger]
    ]
  end

  defp deps do
    [
      {:exqlite, "~> 0.23"},
      {:decimal, "~> 2.0"}
    ]
  end
end
