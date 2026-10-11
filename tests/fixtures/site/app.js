"use strict";

fetch("data.json")
  .then((response) => response.json())
  .then((data) => {
    document.getElementById("routes").textContent = data.routes
      .map((route) => `${route.name}: ${route.km} km`)
      .join("\n");
  });
