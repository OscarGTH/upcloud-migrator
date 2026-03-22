resource "kubernetes_deployment_v1" "scalable-nginx-example" {
  metadata {
    name = "scalable-nginx-example"
    labels = {
      App = "ScalableNginxExample"
    }
  }

  spec {
    replicas = 2

    selector {
      match_labels = {
        App = "ScalableNginxExample"
      }
    }

    template {
      metadata {
        labels = {
          App = "ScalableNginxExample"
        }
      }

      spec {
        container {
          image = "nginx:1.7.8"
          name  = "example"

          port {
            container_port = 80
          }
        }
      }
    }
  }
}
