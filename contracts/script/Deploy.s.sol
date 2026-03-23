// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import {Script, console} from "forge-std/Script.sol";
import {ClaimVerifier} from "../src/ClaimVerifier.sol";

contract Deploy is Script {
    uint64 internal constant DEFAULT_CHALLENGE_PERIOD = 120;
    uint256 internal constant DEFAULT_MIN_BOND = 0.01 ether;

    function run() external {
        vm.startBroadcast();
        ClaimVerifier verifier = new ClaimVerifier(
            DEFAULT_CHALLENGE_PERIOD,
            DEFAULT_MIN_BOND
        );
        vm.stopBroadcast();

        console.log("ClaimVerifier deployed at:", address(verifier));
    }
}
