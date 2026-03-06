// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import "forge-std/Script.sol";
import {ClaimVerifier} from "../src/ClaimVerifier.sol";

contract Deploy is Script {
    function run() external {
        vm.startBroadcast();
        ClaimVerifier verifier = new ClaimVerifier();
        vm.stopBroadcast();

        console.log("ClaimVerifier deployed at:", address(verifier));
    }
}
